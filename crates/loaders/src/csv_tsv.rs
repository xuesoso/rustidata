//! CSV/TSV loading: out-of-core mmap path for plain files, streaming path for
//! compressed files and stdin.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rustidata_core::column::Column;
use rustidata_core::decompress;
use rustidata_core::options::{HeaderMode, Options};
use rustidata_core::sheet::Sheet;
use rustidata_core::sniff;
use rustidata_core::store::{ColumnarStore, MmapTextStore, RowSource};
use rustidata_core::value::Value;

use crate::Notify;

/// Out-of-core path: mmap + background offset index.
pub fn load_mmap(path: &Path, opts: &Options, notify: Notify) -> Result<Sheet> {
    let store = MmapTextStore::open(path, opts.forced_delimiter())?;
    let store = Arc::new(store);

    let columns: Vec<Column> = store
        .headers
        .iter()
        .enumerate()
        .map(|(i, h)| Column::source(h.clone(), i))
        .collect();

    // background indexing of the remainder
    if store.loading().is_some() {
        let st = store.clone();
        std::thread::Builder::new()
            .name("rd-indexer".into())
            .spawn(move || {
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let mut last = std::time::Instant::now();
                st.index_remainder(&cancel, || {
                    if last.elapsed().as_millis() >= 50 {
                        last = std::time::Instant::now();
                        notify();
                    }
                });
                notify();
            })?;
    }

    Ok(Sheet::new(crate::sheet_name(path), store, columns))
}

/// Streaming path for compressed files.
pub fn load_streaming(path: &Path, opts: &Options, notify: Notify) -> Result<Sheet> {
    let reader = decompress::open_reader(path)?;
    load_reader(
        reader,
        crate::sheet_name(path),
        Some(path.to_path_buf()),
        opts,
        notify,
    )
}

/// Stream any reader into a ColumnarStore on a background thread.
pub fn load_reader(
    mut reader: Box<dyn Read + Send>,
    name: String,
    _origin: Option<std::path::PathBuf>,
    opts: &Options,
    notify: Notify,
) -> Result<Sheet> {
    // Sniff from an initial chunk, which we then replay ahead of the rest.
    let mut head = vec![0u8; 64 * 1024];
    let mut filled = 0;
    while filled < head.len() {
        let n = reader.read(&mut head[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    head.truncate(filled);
    let dialect = sniff::sniff(&head, opts.forced_delimiter());
    let has_header = match opts.header {
        HeaderMode::Always => true,
        HeaderMode::Never => false,
        HeaderMode::Auto => dialect.has_header,
    };

    let chained = std::io::Cursor::new(head).chain(reader);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(dialect.delimiter)
        .has_headers(has_header)
        .flexible(true)
        .from_reader(chained);

    let headers: Vec<String> = if has_header {
        rdr.headers()?
            .iter()
            .take(rustidata_core::MAX_COLUMNS)
            .map(|h| h.to_string())
            .collect()
    } else {
        let n = rdr
            .headers()
            .map(|h| h.len())
            .unwrap_or(1)
            .min(rustidata_core::MAX_COLUMNS);
        (0..n).map(|i| format!("col{}", i)).collect()
    };

    let ncols = headers.len().max(1);
    let store = Arc::new(ColumnarStore::new(ncols));
    let columns: Vec<Column> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| Column::source(h.clone(), i))
        .collect();

    let st = store.clone();
    std::thread::Builder::new()
        .name("rd-stream".into())
        .spawn(move || {
            let mut batch: Vec<Vec<Value>> = Vec::with_capacity(16 * 1024);
            let mut record = csv::StringRecord::new();
            let mut bytes_seen = 0u64;
            loop {
                match rdr.read_record(&mut record) {
                    Ok(true) => {
                        bytes_seen += record.as_slice().len() as u64;
                        let row: Vec<Value> = record
                            .iter()
                            .map(|f| {
                                if f.is_empty() {
                                    Value::Null
                                } else {
                                    Value::Str(f.into())
                                }
                            })
                            .collect();
                        batch.push(row);
                        if batch.len() >= 16 * 1024 {
                            st.append_batch(batch.drain(..));
                            st.set_progress(bytes_seen, 0);
                            notify();
                        }
                    }
                    Ok(false) => break,
                    Err(_) => continue, // skip malformed records
                }
            }
            if !batch.is_empty() {
                st.append_batch(batch.drain(..));
            }
            st.finish_loading();
            notify();
        })?;

    Ok(Sheet::new(name, store, columns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn wait_loaded(sheet: &Sheet) {
        for _ in 0..200 {
            if sheet.loading().is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("sheet never finished loading");
    }

    #[test]
    fn mmap_csv() {
        let mut f = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        write!(f, "a,b\n1,2\n3,4\n").unwrap();
        f.flush().unwrap();
        let sheet = load_mmap(f.path(), &Options::default(), Arc::new(|| {})).unwrap();
        wait_loaded(&sheet);
        assert_eq!(sheet.nrows(), 2);
        assert_eq!(sheet.columns.len(), 2);
        assert_eq!(sheet.columns[0].name, "a");
        assert_eq!(sheet.typed_at(1, 1), Value::Str("4".into()));
    }

    #[test]
    fn gzip_streaming() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv.gz");
        let f = std::fs::File::create(&p).unwrap();
        let mut enc = flate2_write_gz(f);
        enc.write_all(b"x,y\n5,6\n7,8\n").unwrap();
        enc.finish().unwrap();

        let sheet = load_streaming(&p, &Options::default(), Arc::new(|| {})).unwrap();
        wait_loaded(&sheet);
        assert_eq!(sheet.nrows(), 2);
        assert_eq!(sheet.typed_at(0, 0), Value::Str("5".into()));
    }

    fn flate2_write_gz(
        f: std::fs::File,
    ) -> rustidata_core::flate2::write::GzEncoder<std::fs::File> {
        rustidata_core::flate2::write::GzEncoder::new(
            f,
            rustidata_core::flate2::Compression::fast(),
        )
    }
}

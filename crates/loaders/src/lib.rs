//! rustidata-loaders: filetype dispatch and format loaders.
//!
//! Routing logic (the hybrid memory model):
//! - plain seekable text (csv/tsv/txt) -> MmapTextStore (out-of-core)
//! - compressed text, stdin, json, xlsx -> ColumnarStore (streamed in
//!   background)
//! - parquet/arrow/sqlite -> native random-access stores (feature-gated)

pub mod csv_tsv;
pub mod fixed;
pub mod json;
pub mod save;
pub mod text;

#[cfg(feature = "parquet")]
pub mod arrow_parquet;
#[cfg(feature = "sqlite")]
pub mod sqlite;
#[cfg(feature = "xlsx")]
pub mod xlsx;

pub mod dir;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustidata_core::decompress::{self, Compression};
use rustidata_core::options::Options;
use rustidata_core::sheet::Sheet;

/// Callback invoked by background loader threads when new rows are available
/// (the TUI sends itself a Dirty message).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Detected filetype after stripping compression suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Csv,
    Tsv,
    Jsonl,
    Json,
    Fixed,
    Txt,
    Parquet,
    Arrow,
    Sqlite,
    Xlsx,
    Dir,
    Unknown,
}

pub fn detect_filetype(path: &Path) -> FileType {
    if path.is_dir() {
        return FileType::Dir;
    }
    let inner = decompress::inner_path(path);
    match inner
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => FileType::Csv,
        Some("tsv") | Some("tab") => FileType::Tsv,
        Some("jsonl") | Some("ndjson") | Some("ldjson") => FileType::Jsonl,
        Some("json") => FileType::Json,
        Some("fixed") => FileType::Fixed,
        Some("txt") | Some("log") => FileType::Txt,
        Some("parquet") | Some("pq") => FileType::Parquet,
        Some("arrow") | Some("arrows") | Some("feather") => FileType::Arrow,
        Some("db") | Some("sqlite") | Some("sqlite3") => FileType::Sqlite,
        Some("xlsx") | Some("xls") | Some("xlsm") => FileType::Xlsx,
        _ => FileType::Unknown,
    }
}

/// Open a path as a Sheet. Background indexing/streaming threads are spawned
/// as needed; they call `notify` when more rows arrive.
pub fn open(path: &Path, opts: &Options, notify: Notify) -> Result<Sheet> {
    let ft = detect_filetype(path);
    let compressed = Compression::from_path(path).is_compressed();

    let mut sheet = match ft {
        FileType::Dir => dir::load_dir(path)?,
        FileType::Csv | FileType::Tsv | FileType::Unknown | FileType::Txt => {
            if compressed {
                csv_tsv::load_streaming(path, opts, notify)?
            } else if ft == FileType::Txt && !looks_tabular(path, opts) {
                text::load_text(path)?
            } else {
                csv_tsv::load_mmap(path, opts, notify)?
            }
        }
        FileType::Jsonl => json::load_jsonl(path, notify)?,
        FileType::Json => json::load_json(path)?,
        FileType::Fixed => fixed::load_fixed(path, notify)?,
        #[cfg(feature = "parquet")]
        FileType::Parquet | FileType::Arrow => arrow_parquet::load(path, ft)?,
        #[cfg(not(feature = "parquet"))]
        FileType::Parquet | FileType::Arrow => anyhow::bail!("built without parquet support"),
        #[cfg(feature = "sqlite")]
        FileType::Sqlite => sqlite::load(path)?,
        #[cfg(not(feature = "sqlite"))]
        FileType::Sqlite => anyhow::bail!("built without sqlite support"),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::load(path)?,
        #[cfg(not(feature = "xlsx"))]
        FileType::Xlsx => anyhow::bail!("built without xlsx support"),
    };
    sheet.source_path = Some(path.to_path_buf());
    Ok(sheet)
}

/// Open stdin as a streamed sheet.
pub fn open_stdin(opts: &Options, notify: Notify) -> Result<Sheet> {
    csv_tsv::load_reader(
        Box::new(std::io::stdin()),
        "stdin".to_string(),
        None,
        opts,
        notify,
    )
}

/// Quick peek: does the first line contain a plausible delimiter?
fn looks_tabular(path: &Path, opts: &Options) -> bool {
    let Ok(mut reader) = decompress::open_reader(path) else {
        return false;
    };
    let mut buf = [0u8; 4096];
    let n = std::io::Read::read(&mut reader, &mut buf).unwrap_or(0);
    let first_line = buf[..n].split(|&b| b == b'\n').next().unwrap_or(&[]);
    if let Some(d) = opts.forced_delimiter() {
        return first_line.contains(&d);
    }
    first_line.contains(&b'\t') || first_line.contains(&b',')
}

/// Sheet name from a path (strip directory and extensions, visidata-style).
pub fn sheet_name(path: &Path) -> String {
    let inner = decompress::inner_path(path);
    inner
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sheet")
        .to_string()
}

pub fn open_with_context(path: &Path, opts: &Options, notify: Notify) -> Result<Sheet> {
    open(path, opts, notify).with_context(|| format!("opening {}", path.display()))
}

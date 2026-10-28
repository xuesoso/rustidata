//! Out-of-core text-table store: mmap + background row-offset index +
//! parse-on-demand with an LRU field cache.
//!
//! This is rustidata's main scalability differentiator over visidata (which
//! materializes every row as Python objects). Opening a multi-GB CSV is
//! near-instant: we sniff the dialect from the first 64KB, index the first
//! ~10K line offsets synchronously so the first screen can render, then a
//! background thread scans the rest of the file with memchr (or a quote-aware
//! state machine when the dialect quotes fields, since quoted fields may
//! contain newlines), appending offsets and publishing progress.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use compact_str::CompactString;
use lru::LruCache;
use memmap2::Mmap;
use parking_lot::{Mutex, RwLock};

use super::{Progress, RawCell, RowId, RowSource};
use crate::sniff::{self, Dialect};

const SNIFF_BYTES: usize = 64 * 1024;
const SYNC_INDEX_ROWS: usize = 10_000;
const FIELD_CACHE_SIZE: usize = 4096;
/// Field-cache shards: keeps rayon workers (sort/freq key extraction) from
/// serializing on a single cache mutex.
const CACHE_SHARDS: usize = 32;

/// Byte offsets of row starts. `starts[i]` is the first byte of row i; the row
/// ends at `starts[i+1] - 1` (or `end_of_data`). Representation kept behind
/// this struct so a delta-compressed scheme can replace the flat Vec later.
pub struct OffsetIndex {
    starts: RwLock<Vec<u64>>,
    complete: AtomicBool,
    bytes_scanned: AtomicU64,
}

impl OffsetIndex {
    fn new() -> Self {
        OffsetIndex {
            starts: RwLock::new(Vec::new()),
            complete: AtomicBool::new(false),
            bytes_scanned: AtomicU64::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.starts.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    /// Byte range of row `i`, or None if not yet indexed.
    fn row_span(&self, i: usize, data_len: u64) -> Option<(u64, u64)> {
        let starts = self.starts.read();
        let start = *starts.get(i)?;
        let end = starts.get(i + 1).copied().unwrap_or(data_len);
        Some((start, end))
    }
}

pub struct MmapTextStore {
    mmap: Mmap,
    /// Offset of the first data row (skips the header line if present).
    data_start: u64,
    pub dialect: Dialect,
    pub headers: Vec<String>,
    ncols: usize,
    index: Arc<OffsetIndex>,
    field_cache: Vec<Mutex<LruCache<RowId, Arc<Vec<CompactString>>>>>,
}

impl MmapTextStore {
    /// Open and sniff a file; synchronously index the first screenful of rows.
    /// Call `index_remainder` on a background thread afterwards.
    pub fn open(path: &Path, forced_delim: Option<u8>) -> std::io::Result<Self> {
        Self::open_with(path, forced_delim, None)
    }

    pub fn open_with(
        path: &Path,
        forced_delim: Option<u8>,
        forced_header: Option<bool>,
    ) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the only unsafe block in rustidata. Mapping a file is unsafe
        // because another process truncating it makes pages past EOF fault
        // (SIGBUS terminates the process — annoying, but not memory
        // corruption). This is the standard tradeoff mmap-based tools accept
        // (ripgrep does the same); growing files are fine since we only read
        // within the mapped length captured at open time.
        let mmap = unsafe { Mmap::map(&file)? };
        #[cfg(unix)]
        let _ = mmap.advise(memmap2::Advice::Sequential);
        let space_fallback = sniff::allows_space_delimiter(path);
        Ok(Self::from_mmap(
            mmap,
            forced_delim,
            forced_header,
            space_fallback,
        ))
    }

    pub fn from_mmap(
        mmap: Mmap,
        forced_delim: Option<u8>,
        forced_header: Option<bool>,
        space_fallback: bool,
    ) -> Self {
        let sample = &mmap[..mmap.len().min(SNIFF_BYTES)];
        let mut dialect = sniff::sniff_with(sample, forced_delim, space_fallback);
        if let Some(h) = forced_header {
            dialect.has_header = h;
        }

        // header row (column count capped for DoS hardening)
        let (headers, data_start) = if dialect.has_header {
            let line_end = memchr::memchr(b'\n', &mmap).unwrap_or(mmap.len());
            let line = String::from_utf8_lossy(trim_cr(&mmap[..line_end]));
            let hdrs: Vec<String> = sniff::split_quoted(&line, dialect.delimiter, dialect.quote)
                .into_iter()
                .take(crate::MAX_COLUMNS)
                .map(|f| unquote(f).to_string())
                .collect();
            (hdrs, (line_end + 1).min(mmap.len()) as u64)
        } else {
            let line_end = memchr::memchr(b'\n', &mmap).unwrap_or(mmap.len());
            let line = String::from_utf8_lossy(trim_cr(&mmap[..line_end]));
            let n = sniff::split_quoted(&line, dialect.delimiter, dialect.quote)
                .len()
                .min(crate::MAX_COLUMNS);
            ((0..n).map(|i| format!("col{}", i)).collect(), 0)
        };

        let ncols = headers.len();
        let store = MmapTextStore {
            mmap,
            data_start,
            dialect,
            headers,
            ncols,
            index: Arc::new(OffsetIndex::new()),
            field_cache: (0..CACHE_SHARDS)
                .map(|_| {
                    Mutex::new(LruCache::new(
                        std::num::NonZeroUsize::new(FIELD_CACHE_SIZE / CACHE_SHARDS).unwrap(),
                    ))
                })
                .collect(),
        };
        store.index_prefix(SYNC_INDEX_ROWS);
        store
    }

    /// Synchronously index up to `max_rows` row starts from data_start.
    fn index_prefix(&self, max_rows: usize) {
        let data = &self.mmap[..];
        let mut starts = self.index.starts.write();
        let mut pos = self.data_start;
        let quote_aware = self.dialect.uses_quotes;
        while (starts.len() < max_rows) && (pos as usize) < data.len() {
            starts.push(pos);
            pos = match next_row_start(
                data,
                pos as usize,
                quote_aware,
                self.dialect.quote,
                self.dialect.delimiter,
            ) {
                Some(p) => p as u64,
                None => {
                    self.finish_index(&mut starts, data.len() as u64);
                    return;
                }
            };
        }
        if (pos as usize) >= data.len() {
            self.finish_index(&mut starts, data.len() as u64);
        }
        self.index.bytes_scanned.store(pos, Ordering::Relaxed);
    }

    fn finish_index(&self, starts: &mut Vec<u64>, data_len: u64) {
        // drop a trailing empty line
        if let Some(&last) = starts.last() {
            if last >= data_len {
                starts.pop();
            }
        }
        self.index.bytes_scanned.store(data_len, Ordering::Relaxed);
        self.index.complete.store(true, Ordering::Release);
    }

    /// Continue indexing the rest of the file. Designed to run on a background
    /// thread; publishes progress through the shared OffsetIndex. `notify` is
    /// called periodically (and once at the end) so the UI can redraw.
    pub fn index_remainder(&self, cancel: &AtomicBool, mut notify: impl FnMut()) {
        let data = &self.mmap[..];
        let data_len = data.len() as u64;
        let quote_aware = self.dialect.uses_quotes;
        let mut pos = {
            let starts = self.index.starts.read();
            if self.index.is_complete() {
                notify();
                return;
            }
            match starts.last() {
                Some(&last) => {
                    match next_row_start(
                        data,
                        last as usize,
                        quote_aware,
                        self.dialect.quote,
                        self.dialect.delimiter,
                    ) {
                        Some(p) => p as u64,
                        None => {
                            drop(starts);
                            let mut w = self.index.starts.write();
                            self.finish_index(&mut w, data_len);
                            notify();
                            return;
                        }
                    }
                }
                None => self.data_start,
            }
        };

        let mut batch: Vec<u64> = Vec::with_capacity(64 * 1024);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            batch.clear();
            // collect a batch without holding the lock
            while batch.len() < 64 * 1024 && (pos as usize) < data.len() {
                batch.push(pos);
                pos = match next_row_start(
                    data,
                    pos as usize,
                    quote_aware,
                    self.dialect.quote,
                    self.dialect.delimiter,
                ) {
                    Some(p) => p as u64,
                    None => data_len,
                };
            }
            if batch.is_empty() {
                let mut w = self.index.starts.write();
                self.finish_index(&mut w, data_len);
                notify();
                return;
            }
            {
                let mut w = self.index.starts.write();
                w.extend_from_slice(&batch);
                if (pos as usize) >= data.len() {
                    self.finish_index(&mut w, data_len);
                    notify();
                    return;
                }
            }
            self.index.bytes_scanned.store(pos, Ordering::Relaxed);
            notify();
        }
    }

    fn fields_for(&self, row: RowId) -> Option<Arc<Vec<CompactString>>> {
        let shard = &self.field_cache[(row as usize) % CACHE_SHARDS];
        if let Some(f) = shard.lock().get(&row) {
            return Some(f.clone());
        }
        let (start, end) = self.index.row_span(row as usize, self.mmap.len() as u64)?;
        let line_bytes = trim_line(&self.mmap[start as usize..end as usize]);
        let line = String::from_utf8_lossy(line_bytes);
        let fields: Vec<CompactString> =
            sniff::split_quoted(&line, self.dialect.delimiter, self.dialect.quote)
                .into_iter()
                .map(|f| CompactString::from(unquote(f)))
                .collect();
        let fields = Arc::new(fields);
        shard.lock().put(row, fields.clone());
        Some(fields)
    }

    /// Raw line text of a row (for TextSheet / debugging).
    pub fn raw_line(&self, row: RowId) -> Option<&[u8]> {
        let (start, end) = self.index.row_span(row as usize, self.mmap.len() as u64)?;
        Some(trim_line(&self.mmap[start as usize..end as usize]))
    }

    pub fn file_len(&self) -> u64 {
        self.mmap.len() as u64
    }

    pub fn index_handle(&self) -> Arc<OffsetIndex> {
        self.index.clone()
    }
}

impl RowSource for MmapTextStore {
    fn len(&self) -> u64 {
        self.index.len() as u64
    }

    fn loading(&self) -> Option<Progress> {
        if self.index.is_complete() {
            None
        } else {
            Some(Progress {
                done: self.index.bytes_scanned.load(Ordering::Relaxed),
                total: Some(self.mmap.len() as u64),
                gerund: "indexing",
            })
        }
    }

    fn ncols(&self) -> usize {
        self.ncols
    }

    fn raw(&self, row: RowId, col: usize) -> RawCell {
        match self.fields_for(row) {
            Some(fields) => match fields.get(col) {
                Some(f) => RawCell::Text(f.clone()),
                None => RawCell::Missing,
            },
            None => RawCell::Missing,
        }
    }

    fn raw_row(&self, row: RowId, out: &mut Vec<RawCell>) {
        out.clear();
        if let Some(fields) = self.fields_for(row) {
            for c in 0..self.ncols {
                out.push(match fields.get(c) {
                    Some(f) => RawCell::Text(f.clone()),
                    None => RawCell::Missing,
                });
            }
        } else {
            out.resize(self.ncols, RawCell::Missing);
        }
    }
}

/// Find the byte offset of the next row start after the row beginning at
/// `from`. Fast path: memchr for '\n'. Quote-aware path: state machine that
/// skips newlines inside quoted fields. As in `sniff::split_quoted`, a quote
/// opens a quoted field only at the start of a field; a bare quote mid-field
/// (`55" screen`) is literal, so it can't swallow the rest of the file.
fn next_row_start(
    data: &[u8],
    from: usize,
    quote_aware: bool,
    quote: Option<u8>,
    delim: u8,
) -> Option<usize> {
    if !quote_aware {
        let nl = memchr::memchr(b'\n', &data[from..])?;
        let next = from + nl + 1;
        return (next < data.len()).then_some(next);
    }
    let q = quote.unwrap_or(b'"');
    let mut in_quotes = false;
    let mut i = from;
    // memchr2 jumps between the only bytes that matter (quote / newline)
    // instead of walking every byte; this loop is the indexing hot path for
    // quoted files. Field starts are found by looking one byte back rather
    // than stopping at every delimiter: outside quotes, a quote begins a field
    // exactly when it is at the row start or right after a delimiter.
    while let Some(off) = memchr::memchr2(q, b'\n', &data[i..]) {
        let j = i + off;
        if in_quotes {
            if data[j] == q {
                if data.get(j + 1) == Some(&q) {
                    i = j + 2; // escaped "" stays inside the quotes
                    continue;
                }
                in_quotes = false;
            }
        } else if data[j] == b'\n' {
            let next = j + 1;
            return (next < data.len()).then_some(next);
        } else if j == from || data[j - 1] == delim {
            in_quotes = true;
        }
        i = j + 1;
    }
    None
}

fn trim_cr(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

fn trim_line(span: &[u8]) -> &[u8] {
    let span = match span.last() {
        Some(b'\n') => &span[..span.len() - 1],
        _ => span,
    };
    trim_cr(span)
}

/// Strip surrounding quotes and collapse doubled quotes.
fn unquote(f: &str) -> std::borrow::Cow<'_, str> {
    let b = f.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        let inner = &f[1..f.len() - 1];
        if inner.contains("\"\"") {
            return std::borrow::Cow::Owned(inner.replace("\"\"", "\""));
        }
        return std::borrow::Cow::Borrowed(inner);
    }
    std::borrow::Cow::Borrowed(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn store_from(content: &str) -> MmapTextStore {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        let store = MmapTextStore::open(f.path(), None).unwrap();
        // index everything synchronously for tests
        store.index_remainder(&AtomicBool::new(false), || {});
        store
    }

    #[test]
    fn basic_csv() {
        let s = store_from("a,b,c\n1,2,3\n4,5,6\n");
        assert_eq!(s.headers, vec!["a", "b", "c"]);
        assert_eq!(s.len(), 2);
        assert_eq!(s.raw(0, 0).as_str(), Some("1"));
        assert_eq!(s.raw(1, 2).as_str(), Some("6"));
        assert!(s.loading().is_none());
    }

    #[test]
    fn headerless() {
        let s = store_from("1,2\n3,4\n");
        assert_eq!(s.headers, vec!["col0", "col1"]);
        assert_eq!(s.len(), 2);
        assert_eq!(s.raw(0, 0).as_str(), Some("1"));
    }

    #[test]
    fn quoted_embedded_newline() {
        let s = store_from("a,b\n\"line1\nline2\",x\nfoo,y\n");
        assert_eq!(s.len(), 2);
        assert_eq!(s.raw(0, 0).as_str(), Some("line1\nline2"));
        assert_eq!(s.raw(1, 0).as_str(), Some("foo"));
    }

    #[test]
    fn crlf_lines() {
        let s = store_from("a,b\r\n1,2\r\n3,4\r\n");
        assert_eq!(s.headers, vec!["a", "b"]);
        assert_eq!(s.raw(0, 1).as_str(), Some("2"));
    }

    #[test]
    fn doubled_quotes() {
        let s = store_from("a,b\n\"say \"\"hi\"\"\",2\n");
        assert_eq!(s.raw(0, 0).as_str(), Some("say \"hi\""));
    }

    #[test]
    fn ragged_rows() {
        let s = store_from("a,b,c\n1,2\n");
        assert_eq!(s.raw(0, 2), RawCell::Missing);
    }

    #[test]
    fn no_trailing_newline() {
        let s = store_from("a,b\n1,2\n3,4");
        assert_eq!(s.len(), 2);
        assert_eq!(s.raw(1, 1).as_str(), Some("4"));
    }

    fn store_with_name(name: &str, content: &str) -> MmapTextStore {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(name);
        std::fs::write(&p, content).unwrap();
        let store = MmapTextStore::open(&p, None).unwrap();
        store.index_remainder(&AtomicBool::new(false), || {});
        store
    }

    #[test]
    fn space_delimited_only_for_tsv_extension() {
        let s = store_with_name("a.tsv", "chrom start end\nchr1 0 100\n");
        assert_eq!(s.headers, vec!["chrom", "start", "end"]);
        assert_eq!(s.raw(0, 2).as_str(), Some("100"));

        let s = store_with_name("a.csv", "full name\nJohn Smith\nJane Doe\n");
        assert_eq!(s.headers, vec!["full name"]);
        assert_eq!(s.raw(1, 0).as_str(), Some("Jane Doe"));
    }

    #[test]
    fn tsv_detected() {
        let s = store_from("x\ty\n1\t2\n");
        assert_eq!(s.dialect.delimiter, b'\t');
        assert_eq!(s.raw(0, 1).as_str(), Some("2"));
    }

    #[test]
    fn tsv_bare_quotes_mid_field_are_literal() {
        // Regression: a bare `"` (inch mark) used to switch on quote-aware
        // indexing and merge every following row into one.
        let s = store_from("item\tsize\nTV\t55\" screen\nphone\t6\"\nlaptop\t15\n");
        assert_eq!(s.len(), 3);
        assert_eq!(s.raw(0, 1).as_str(), Some("55\" screen"));
        assert_eq!(s.raw(1, 0).as_str(), Some("phone"));
        assert_eq!(s.raw(1, 1).as_str(), Some("6\""));
        assert_eq!(s.raw(2, 1).as_str(), Some("15"));
    }

    #[test]
    fn stray_quote_does_not_swallow_large_file() {
        let mut content = String::from("id\tnote\n1\t12\" ruler\n");
        for i in 2..=25_000 {
            content.push_str(&format!("{}\tplain\n", i));
        }
        let s = store_from(&content);
        assert_eq!(s.len(), 25_000);
        assert_eq!(s.raw(24_999, 0).as_str(), Some("25000"));
    }

    #[test]
    fn mixed_quoted_fields_and_bare_quotes() {
        // field-start quotes still quote (embedded tab/newline/escaped quote),
        // while a mid-field quote in the same file stays literal
        let s = store_from("a\tb\n\"x\ty\nz\"\t1\n5\" pipe\t2\n\"say \"\"hi\"\"\"\t3\n");
        assert_eq!(s.len(), 3);
        assert_eq!(s.raw(0, 0).as_str(), Some("x\ty\nz"));
        assert_eq!(s.raw(1, 0).as_str(), Some("5\" pipe"));
        assert_eq!(s.raw(2, 0).as_str(), Some("say \"hi\""));
        assert_eq!(s.raw(2, 1).as_str(), Some("3"));
    }

    #[test]
    fn csv_mid_field_quote_with_quoted_rows() {
        let s = store_from("a,b\n\"p\nq\",1\n12\" ruler,2\nlast,3\n");
        assert_eq!(s.len(), 3);
        assert_eq!(s.raw(1, 0).as_str(), Some("12\" ruler"));
        assert_eq!(s.raw(2, 1).as_str(), Some("3"));
    }

    #[test]
    fn quote_state_machine_across_many_rows() {
        // mixed quoted/unquoted rows, quoted newlines, doubled quotes, and a
        // quoted field ending the file — pins the memchr2 state machine
        let s = store_from(
            "a,b\n\
             \"x\ny\nz\",1\n\
             plain,2\n\
             \"say \"\"hi\"\"\n bye\",3\n\
             last,\"tail\"\n",
        );
        assert_eq!(s.len(), 4);
        assert_eq!(s.raw(0, 0).as_str(), Some("x\ny\nz"));
        assert_eq!(s.raw(1, 0).as_str(), Some("plain"));
        assert_eq!(s.raw(2, 1).as_str(), Some("3"));
        assert_eq!(s.raw(3, 1).as_str(), Some("tail"));
    }

    #[test]
    fn matches_naive_line_split_on_large_input() {
        let mut content = String::from("h0,h1\n");
        for i in 0..25_000 {
            content.push_str(&format!("v{},w{}\n", i, i));
        }
        let s = store_from(&content);
        assert_eq!(s.len(), 25_000);
        assert_eq!(s.raw(24_999, 1).as_str(), Some("w24999"));
        assert_eq!(s.raw(13_000, 0).as_str(), Some("v13000"));
    }
}

//! Row storage backends.
//!
//! A `RowSource` provides random access to raw cell data. The two main
//! implementations are `MmapTextStore` (out-of-core, parse-on-demand over a
//! memory-mapped text file) and `ColumnarStore` (in-memory typed columns, used
//! for streamed/derived data).

pub mod columnar;
pub mod mmap_text;
pub mod overlay;

use crate::value::Value;

pub use columnar::ColumnarStore;
pub use mmap_text::MmapTextStore;
pub use overlay::Overlay;

/// Stable row identifier within a source. Survives sorting (sorting produces a
/// permutation of RowIds, never moves data).
pub type RowId = u64;

/// A raw cell as the store holds it, before column typing.
#[derive(Debug, Clone, PartialEq)]
pub enum RawCell {
    /// Field absent in this row (ragged data).
    Missing,
    /// Owned, already-typed value (columnar stores).
    Owned(Value),
    /// Borrowed text (mmap stores yield these without copying).
    Text(compact_str::CompactString),
}

impl RawCell {
    /// Realize as an untyped Value.
    pub fn into_value(self) -> Value {
        match self {
            RawCell::Missing => Value::Null,
            RawCell::Owned(v) => v,
            RawCell::Text(s) => {
                if s.is_empty() {
                    Value::Null
                } else {
                    Value::Str(s)
                }
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            RawCell::Text(s) => Some(s),
            RawCell::Owned(Value::Str(s)) => Some(s),
            _ => None,
        }
    }
}

/// Loading progress reported by a store still being indexed/ingested.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    pub gerund: &'static str,
}

impl Progress {
    pub fn pct(&self) -> Option<u8> {
        let total = self.total?;
        if total == 0 {
            return None;
        }
        Some(((self.done as f64 / total as f64) * 100.0).min(100.0) as u8)
    }
}

/// Random-access source of raw rows. Implementations must be cheap to call
/// from both the UI thread and rayon workers.
pub trait RowSource: Send + Sync {
    /// Rows available *so far* (grows while a background indexer runs).
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `Some(progress)` while loading/indexing is still under way.
    fn loading(&self) -> Option<Progress>;

    /// Number of columns the source exposes.
    fn ncols(&self) -> usize;

    /// Fetch one raw cell. Out-of-range rows/cols return `RawCell::Missing`.
    fn raw(&self, row: RowId, col: usize) -> RawCell;

    /// Fetch a whole raw row; default implementation calls `raw` per column,
    /// stores can override to avoid re-finding fields.
    fn raw_row(&self, row: RowId, out: &mut Vec<RawCell>) {
        out.clear();
        for c in 0..self.ncols() {
            out.push(self.raw(row, c));
        }
    }
}

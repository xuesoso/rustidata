//! In-memory typed columnar store.
//!
//! Used for streamed sources (gzip, stdin), formats without random access
//! (xlsx, json), and all derived sheets (freq/pivot/join results). Loaders
//! append row batches on a background thread under short write locks while the
//! UI reads already-released rows.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use compact_str::CompactString;
use parking_lot::RwLock;
use roaring::RoaringBitmap;

use super::{Progress, RawCell, RowId, RowSource};
use crate::value::Value;

/// One typed column of data. `Str` packs all strings into an arena.
#[derive(Debug)]
pub enum ColumnData {
    Str {
        arena: Vec<u8>,
        /// offsets.len() == nrows + 1
        offsets: Vec<u32>,
        nulls: RoaringBitmap,
    },
    Int {
        data: Vec<i64>,
        nulls: RoaringBitmap,
    },
    Float {
        data: Vec<f64>,
        nulls: RoaringBitmap,
    },
    Date {
        data: Vec<i64>,
        nulls: RoaringBitmap,
    },
    Bool {
        data: Vec<bool>,
        nulls: RoaringBitmap,
    },
    /// Fallback for mixed-type columns (JSON etc).
    Any(Vec<Value>),
}

impl ColumnData {
    pub fn new_str() -> Self {
        ColumnData::Str {
            arena: Vec::new(),
            offsets: vec![0],
            nulls: RoaringBitmap::new(),
        }
    }

    pub fn new_any() -> Self {
        ColumnData::Any(Vec::new())
    }

    pub fn len(&self) -> usize {
        match self {
            ColumnData::Str { offsets, .. } => offsets.len() - 1,
            ColumnData::Int { data, .. } => data.len(),
            ColumnData::Float { data, .. } => data.len(),
            ColumnData::Date { data, .. } => data.len(),
            ColumnData::Bool { data, .. } => data.len(),
            ColumnData::Any(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, row: usize) -> RawCell {
        match self {
            ColumnData::Str {
                arena,
                offsets,
                nulls,
            } => {
                if row + 1 >= offsets.len() {
                    return RawCell::Missing;
                }
                if nulls.contains(row as u32) {
                    return RawCell::Owned(Value::Null);
                }
                let s = &arena[offsets[row] as usize..offsets[row + 1] as usize];
                RawCell::Text(CompactString::from(std::str::from_utf8(s).unwrap_or("")))
            }
            ColumnData::Int { data, nulls } => match data.get(row) {
                None => RawCell::Missing,
                Some(_) if nulls.contains(row as u32) => RawCell::Owned(Value::Null),
                Some(v) => RawCell::Owned(Value::Int(*v)),
            },
            ColumnData::Float { data, nulls } => match data.get(row) {
                None => RawCell::Missing,
                Some(_) if nulls.contains(row as u32) => RawCell::Owned(Value::Null),
                Some(v) => RawCell::Owned(Value::Float(*v)),
            },
            ColumnData::Date { data, nulls } => match data.get(row) {
                None => RawCell::Missing,
                Some(_) if nulls.contains(row as u32) => RawCell::Owned(Value::Null),
                Some(v) => RawCell::Owned(Value::Date(*v)),
            },
            ColumnData::Bool { data, nulls } => match data.get(row) {
                None => RawCell::Missing,
                Some(_) if nulls.contains(row as u32) => RawCell::Owned(Value::Null),
                Some(v) => RawCell::Owned(Value::Bool(*v)),
            },
            ColumnData::Any(v) => match v.get(row) {
                None => RawCell::Missing,
                Some(val) => RawCell::Owned(val.clone()),
            },
        }
    }

    /// Append a value, converting to this column's representation.
    pub fn push(&mut self, v: Value) {
        match self {
            ColumnData::Str {
                arena,
                offsets,
                nulls,
            } => {
                let row = (offsets.len() - 1) as u32;
                if v.is_null() {
                    nulls.insert(row);
                } else {
                    let s = v.to_display();
                    arena.extend_from_slice(s.as_bytes());
                }
                offsets.push(arena.len() as u32);
            }
            ColumnData::Int { data, nulls } => {
                let row = data.len() as u32;
                match v.as_i64() {
                    Some(i) if !v.is_null() => data.push(i),
                    _ => {
                        nulls.insert(row);
                        data.push(0);
                    }
                }
            }
            ColumnData::Float { data, nulls } => {
                let row = data.len() as u32;
                match v.as_f64() {
                    Some(f) if !v.is_null() => data.push(f),
                    _ => {
                        nulls.insert(row);
                        data.push(0.0);
                    }
                }
            }
            ColumnData::Date { data, nulls } => {
                let row = data.len() as u32;
                match v {
                    Value::Date(us) => data.push(us),
                    _ => {
                        nulls.insert(row);
                        data.push(0);
                    }
                }
            }
            ColumnData::Bool { data, nulls } => {
                let row = data.len() as u32;
                match v {
                    Value::Bool(b) => data.push(b),
                    _ => {
                        nulls.insert(row);
                        data.push(false);
                    }
                }
            }
            ColumnData::Any(vals) => vals.push(v),
        }
    }
}

/// Growable columnar store. `nrows` is published only after a whole batch has
/// been appended, so readers never see a partially-written row.
pub struct ColumnarStore {
    cols: Vec<RwLock<ColumnData>>,
    /// Extra columns appended after creation (e.g. JSON discovering new keys).
    /// Kept in a separate growable list so `cols` indices stay stable.
    nrows: AtomicU64,
    loading: AtomicBool,
    loaded_bytes: AtomicU64,
    total_bytes: AtomicU64,
}

impl ColumnarStore {
    pub fn new(ncols: usize) -> Self {
        ColumnarStore {
            cols: (0..ncols).map(|_| RwLock::new(ColumnData::new_str())).collect(),
            nrows: AtomicU64::new(0),
            loading: AtomicBool::new(true),
            loaded_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
        }
    }

    pub fn with_columns(cols: Vec<ColumnData>) -> Self {
        let nrows = cols.first().map(|c| c.len() as u64).unwrap_or(0);
        ColumnarStore {
            cols: cols.into_iter().map(RwLock::new).collect(),
            nrows: AtomicU64::new(nrows),
            loading: AtomicBool::new(false),
            loaded_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
        }
    }

    /// Build a small store from rows of Values (derived sheets, tests).
    pub fn from_rows(ncols: usize, rows: impl IntoIterator<Item = Vec<Value>>) -> Self {
        let store = ColumnarStore::new_any(ncols);
        store.append_batch(rows);
        store.finish_loading();
        store
    }

    pub fn new_any(ncols: usize) -> Self {
        ColumnarStore {
            cols: (0..ncols).map(|_| RwLock::new(ColumnData::new_any())).collect(),
            nrows: AtomicU64::new(0),
            loading: AtomicBool::new(true),
            loaded_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
        }
    }

    /// Append a batch of rows. Rows shorter than ncols are padded with Null.
    pub fn append_batch(&self, rows: impl IntoIterator<Item = Vec<Value>>) {
        let mut count = 0u64;
        let mut guards: Vec<_> = self.cols.iter().map(|c| c.write()).collect();
        for row in rows {
            let mut it = row.into_iter();
            for g in guards.iter_mut() {
                g.push(it.next().unwrap_or(Value::Null));
            }
            count += 1;
        }
        drop(guards);
        self.nrows.fetch_add(count, Ordering::Release);
    }

    pub fn set_progress(&self, done: u64, total: u64) {
        self.loaded_bytes.store(done, Ordering::Relaxed);
        self.total_bytes.store(total, Ordering::Relaxed);
    }

    pub fn finish_loading(&self) {
        self.loading.store(false, Ordering::Release);
    }
}

impl RowSource for ColumnarStore {
    fn len(&self) -> u64 {
        self.nrows.load(Ordering::Acquire)
    }

    fn loading(&self) -> Option<Progress> {
        if self.loading.load(Ordering::Acquire) {
            let total = self.total_bytes.load(Ordering::Relaxed);
            Some(Progress {
                done: self.loaded_bytes.load(Ordering::Relaxed),
                total: (total > 0).then_some(total),
                gerund: "loading",
            })
        } else {
            None
        }
    }

    fn ncols(&self) -> usize {
        self.cols.len()
    }

    fn raw(&self, row: RowId, col: usize) -> RawCell {
        if row >= self.len() {
            return RawCell::Missing;
        }
        match self.cols.get(col) {
            Some(c) => c.read().get(row as usize),
            None => RawCell::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read() {
        let store = ColumnarStore::new(2);
        store.append_batch(vec![
            vec![Value::Str("a".into()), Value::Str("1".into())],
            vec![Value::Str("b".into())], // ragged: padded with null
        ]);
        store.finish_loading();
        assert_eq!(store.len(), 2);
        assert_eq!(store.raw(0, 0).as_str(), Some("a"));
        assert_eq!(store.raw(1, 1), RawCell::Owned(Value::Null));
        assert_eq!(store.raw(5, 0), RawCell::Missing);
        assert!(store.loading().is_none());
    }

    #[test]
    fn typed_columns() {
        let mut col = ColumnData::Int {
            data: vec![],
            nulls: RoaringBitmap::new(),
        };
        col.push(Value::Int(5));
        col.push(Value::Null);
        let store = ColumnarStore::with_columns(vec![col]);
        assert_eq!(store.raw(0, 0), RawCell::Owned(Value::Int(5)));
        assert_eq!(store.raw(1, 0), RawCell::Owned(Value::Null));
    }
}

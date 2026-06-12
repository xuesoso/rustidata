//! Mutation overlay: cell edits, row deletions, and row insertions layered
//! over any (possibly immutable, e.g. mmap-backed) RowSource.
//!
//! Read order: edits -> inserted rows -> source. RowIds for inserted rows have
//! the high bit set so they never collide with source rows; selection bitmaps
//! and sort permutations work uniformly across both.

use std::collections::HashMap;

use roaring::RoaringTreemap;

use super::{RawCell, RowId, RowSource};
use crate::value::Value;

/// RowIds at or above this bound denote inserted rows.
pub const INSERT_BASE: RowId = 1 << 63;

#[derive(Default)]
pub struct Overlay {
    pub edits: HashMap<(RowId, usize), Value>,
    pub deleted: RoaringTreemap,
    /// Inserted rows, indexed by (rowid - INSERT_BASE).
    inserted: Vec<Vec<Value>>,
}

impl Overlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_inserted(row: RowId) -> bool {
        row >= INSERT_BASE
    }

    pub fn set_cell(&mut self, row: RowId, col: usize, v: Value) -> Option<Value> {
        if let Some(idx) = row.checked_sub(INSERT_BASE) {
            let r = self.inserted.get_mut(idx as usize)?;
            if col >= r.len() {
                r.resize(col + 1, Value::Null);
            }
            let old = std::mem::replace(&mut r[col], v);
            return Some(old);
        }
        self.edits.insert((row, col), v)
    }

    /// Remove an edit, restoring the underlying source value (undo support).
    pub fn clear_cell(&mut self, row: RowId, col: usize) {
        self.edits.remove(&(row, col));
    }

    pub fn delete_row(&mut self, row: RowId) {
        self.deleted.insert(row);
    }

    pub fn undelete_row(&mut self, row: RowId) {
        self.deleted.remove(row);
    }

    pub fn is_deleted(&self, row: RowId) -> bool {
        self.deleted.contains(row)
    }

    /// Insert a new row, returning its RowId.
    pub fn insert_row(&mut self, values: Vec<Value>) -> RowId {
        let id = INSERT_BASE + self.inserted.len() as RowId;
        self.inserted.push(values);
        id
    }

    pub fn n_inserted(&self) -> u64 {
        self.inserted.len() as u64
    }

    pub fn n_deleted(&self) -> u64 {
        self.deleted.len()
    }

    pub fn has_changes(&self) -> bool {
        !self.edits.is_empty() || !self.deleted.is_empty() || !self.inserted.is_empty()
    }

    /// Read a cell through the overlay.
    pub fn cell(&self, source: &dyn RowSource, row: RowId, col: usize) -> RawCell {
        if let Some(v) = self.edits.get(&(row, col)) {
            return RawCell::Owned(v.clone());
        }
        if let Some(idx) = row.checked_sub(INSERT_BASE) {
            return match self.inserted.get(idx as usize).and_then(|r| r.get(col)) {
                Some(v) => RawCell::Owned(v.clone()),
                None => RawCell::Missing,
            };
        }
        source.raw(row, col)
    }

    /// Snapshot for use by background jobs (sort/aggregate) while the UI may
    /// keep mutating the live overlay.
    pub fn snapshot(&self) -> Overlay {
        Overlay {
            edits: self.edits.clone(),
            deleted: self.deleted.clone(),
            inserted: self.inserted.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ColumnarStore;

    fn base() -> ColumnarStore {
        ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Int(1), Value::Str("a".into())],
                vec![Value::Int(2), Value::Str("b".into())],
            ],
        )
    }

    #[test]
    fn edit_precedence() {
        let src = base();
        let mut ov = Overlay::new();
        assert_eq!(ov.cell(&src, 0, 0), RawCell::Owned(Value::Int(1)));
        ov.set_cell(0, 0, Value::Int(99));
        assert_eq!(ov.cell(&src, 0, 0), RawCell::Owned(Value::Int(99)));
        ov.clear_cell(0, 0);
        assert_eq!(ov.cell(&src, 0, 0), RawCell::Owned(Value::Int(1)));
    }

    #[test]
    fn insert_and_delete() {
        let src = base();
        let mut ov = Overlay::new();
        let id = ov.insert_row(vec![Value::Int(3), Value::Str("c".into())]);
        assert!(Overlay::is_inserted(id));
        assert_eq!(ov.cell(&src, id, 1), RawCell::Owned(Value::Str("c".into())));
        ov.delete_row(1);
        assert!(ov.is_deleted(1));
        ov.undelete_row(1);
        assert!(!ov.is_deleted(1));
    }

    #[test]
    fn edit_inserted_row() {
        let src = base();
        let mut ov = Overlay::new();
        let id = ov.insert_row(vec![Value::Null]);
        ov.set_cell(id, 0, Value::Int(7));
        assert_eq!(ov.cell(&src, id, 0), RawCell::Owned(Value::Int(7)));
    }
}

//! Closure-based undo/redo, in the spirit of visidata's addUndo.

use crate::store::RowId;
use crate::value::Value;

/// A reversible mutation. Enum (not closures) so ops are inspectable and the
/// inverse relationship is enforced by construction.
#[derive(Clone)]
pub enum UndoOp {
    /// old=None means the overlay had no edit for this cell before.
    SetCell {
        row: RowId,
        col: usize,
        old: Option<Value>,
        new: Value,
    },
    DeleteRows { rows: Vec<RowId> },
    /// Selection state before/after a bulk selection change.
    SetSelection {
        old: roaring::RoaringTreemap,
        new: roaring::RoaringTreemap,
    },
    /// Sort permutation change.
    SetPerm {
        old: Option<std::sync::Arc<Vec<RowId>>>,
        new: Option<std::sync::Arc<Vec<RowId>>>,
        old_spec: Vec<(usize, bool)>,
        new_spec: Vec<(usize, bool)>,
    },
    /// Full column-list snapshot (rename/move/hide/type changes).
    SetColumns {
        old: Vec<crate::column::Column>,
        new: Vec<crate::column::Column>,
    },
}

pub struct UndoEntry {
    pub label: String,
    pub ops: Vec<UndoOp>,
}

#[derive(Default)]
pub struct UndoStack {
    done: Vec<UndoEntry>,
    undone: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, label: impl Into<String>, ops: Vec<UndoOp>) {
        if ops.is_empty() {
            return;
        }
        self.done.push(UndoEntry {
            label: label.into(),
            ops,
        });
        self.undone.clear();
    }

    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        let e = self.done.pop()?;
        Some(e)
    }

    pub fn push_undone(&mut self, e: UndoEntry) {
        self.undone.push(e);
    }

    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        self.undone.pop()
    }

    pub fn push_redone(&mut self, e: UndoEntry) {
        self.done.push(e);
    }

    pub fn n_undoable(&self) -> usize {
        self.done.len()
    }

    pub fn n_redoable(&self) -> usize {
        self.undone.len()
    }
}

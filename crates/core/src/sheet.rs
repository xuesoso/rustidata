//! The Sheet: columns + row source + overlay + view state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use roaring::RoaringTreemap;

use crate::column::Column;
use crate::store::{overlay::Overlay, Progress, RawCell, RowId, RowSource};
use crate::undo::UndoStack;
use crate::value::Value;

pub type SheetId = u64;

static NEXT_SHEET_ID: AtomicU64 = AtomicU64::new(1);

/// Sheet flavor; drives keybinding lookup and per-kind behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SheetKind {
    Table,
    Freq,
    Pivot,
    Describe,
    Melt,
    Join,
    Transpose,
    Unfurl,
    Graph,
    Text,
    Dir,
    Sheets,
    Columns,
    Options,
    Help,
    Errors,
}

impl SheetKind {
    /// Parent kind for keybinding fallback (mimics Python MRO).
    pub fn parent(&self) -> Option<SheetKind> {
        match self {
            SheetKind::Table => None,
            SheetKind::Graph | SheetKind::Text => None,
            _ => Some(SheetKind::Table),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ViewState {
    /// Cursor position as a *view index* (0-based among visible rows).
    pub cur_row: u64,
    /// Cursor position among visible columns.
    pub cur_col: usize,
    /// First visible row (view index).
    pub top_row: u64,
    /// First visible non-key column (index into visible_cols).
    pub left_col: usize,
}

pub struct Sheet {
    pub id: SheetId,
    pub name: String,
    pub kind: SheetKind,
    pub source: Arc<dyn RowSource>,
    pub overlay: Overlay,
    pub columns: Vec<Column>,
    /// Sort permutation of RowIds. None = natural order.
    pub perm: Option<Arc<Vec<RowId>>>,
    /// Selected rows, keyed by RowId (survives sorting).
    pub selection: RoaringTreemap,
    /// (column index into `columns`, descending) — active sort spec.
    pub sort_spec: Vec<(usize, bool)>,
    pub view: ViewState,
    pub undo: UndoStack,
    /// For derived sheets: the sheet this was derived from.
    pub parent: Option<SheetId>,
    /// For derived sheets: source RowIds backing each derived row.
    pub groups: Option<Arc<Vec<Vec<RowId>>>>,
    /// Path this sheet was loaded from (for save/reload).
    pub source_path: Option<std::path::PathBuf>,
}

impl Sheet {
    pub fn new(
        name: impl Into<String>,
        source: Arc<dyn RowSource>,
        columns: Vec<Column>,
    ) -> Self {
        Sheet {
            id: NEXT_SHEET_ID.fetch_add(1, Ordering::Relaxed),
            name: name.into(),
            kind: SheetKind::Table,
            source,
            overlay: Overlay::new(),
            columns,
            perm: None,
            selection: RoaringTreemap::new(),
            sort_spec: Vec::new(),
            view: ViewState::default(),
            undo: UndoStack::new(),
            parent: None,
            groups: None,
            source_path: None,
        }
    }

    pub fn with_kind(mut self, kind: SheetKind) -> Self {
        self.kind = kind;
        self
    }

    /// Raw cell through the overlay.
    pub fn raw_cell(&self, row: RowId, col: usize) -> RawCell {
        self.overlay.cell(self.source.as_ref(), row, col)
    }

    pub fn loading(&self) -> Option<Progress> {
        self.source.loading()
    }

    /// Number of visible rows (after deletions / under permutation).
    pub fn nrows(&self) -> u64 {
        match &self.perm {
            Some(p) => p.len() as u64,
            None => {
                self.source.len() + self.overlay.n_inserted() - self.overlay.n_deleted()
            }
        }
    }

    /// Map a view index to a stable RowId.
    pub fn rowid_at(&self, view_idx: u64) -> Option<RowId> {
        if view_idx >= self.nrows() {
            return None;
        }
        if let Some(p) = &self.perm {
            return p.get(view_idx as usize).copied();
        }
        let n_deleted = self.overlay.n_deleted();
        let src_visible = self.source.len() - n_deleted;
        if view_idx >= src_visible {
            // inserted rows come after all source rows in natural order
            return Some(crate::store::overlay::INSERT_BASE + (view_idx - src_visible));
        }
        if n_deleted == 0 {
            return Some(view_idx);
        }
        // n-th non-deleted source row: binary search on rank
        let n = view_idx;
        let mut lo = n;
        let mut hi = n + n_deleted;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let visible_through_mid = mid + 1 - self.overlay.deleted.rank(mid);
            if visible_through_mid <= n {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }

    /// Inverse of `rowid_at` for natural (unsorted) order; with a permutation
    /// it scans (used by search-from-cursor on sorted sheets).
    pub fn view_index_of(&self, rowid: RowId) -> Option<u64> {
        match &self.perm {
            Some(p) => p.iter().position(|&r| r == rowid).map(|i| i as u64),
            None => {
                if self.overlay.is_deleted(rowid) {
                    return None;
                }
                if let Some(ins) = rowid.checked_sub(crate::store::overlay::INSERT_BASE) {
                    let src_visible = self.source.len() - self.overlay.n_deleted();
                    return Some(src_visible + ins);
                }
                Some(rowid - self.overlay.deleted.rank(rowid))
            }
        }
    }

    /// The RowId under the cursor.
    pub fn cursor_rowid(&self) -> Option<RowId> {
        self.rowid_at(self.view.cur_row)
    }

    /// Visible columns in display order: key columns first (by key_pos), then
    /// the rest in `columns` order, skipping hidden.
    pub fn visible_cols(&self) -> Vec<usize> {
        let mut keys: Vec<usize> = (0..self.columns.len())
            .filter(|&i| self.columns[i].is_key() && !self.columns[i].hidden)
            .collect();
        keys.sort_by_key(|&i| self.columns[i].key_pos);
        let rest = (0..self.columns.len())
            .filter(|&i| !self.columns[i].is_key() && !self.columns[i].hidden);
        keys.extend(rest);
        keys
    }

    pub fn n_key_cols(&self) -> usize {
        self.columns
            .iter()
            .filter(|c| c.is_key() && !c.hidden)
            .count()
    }

    /// The column (index into `columns`) under the cursor.
    pub fn cursor_col_index(&self) -> Option<usize> {
        self.visible_cols().get(self.view.cur_col).copied()
    }

    pub fn cursor_column(&self) -> Option<&Column> {
        self.cursor_col_index().map(|i| &self.columns[i])
    }

    /// Typed value at (view row, columns index).
    pub fn typed_at(&self, view_idx: u64, col: usize) -> Value {
        match self.rowid_at(view_idx) {
            Some(rid) => self.columns[col].typed_value(self, rid),
            None => Value::Null,
        }
    }

    // ---- cursor movement (clamped) ----

    pub fn move_cursor_rows(&mut self, delta: i64) {
        let n = self.nrows();
        if n == 0 {
            self.view.cur_row = 0;
            return;
        }
        let cur = self.view.cur_row as i64 + delta;
        self.view.cur_row = cur.clamp(0, n as i64 - 1) as u64;
    }

    pub fn move_cursor_cols(&mut self, delta: i64) {
        let n = self.visible_cols().len();
        if n == 0 {
            self.view.cur_col = 0;
            return;
        }
        let cur = self.view.cur_col as i64 + delta;
        self.view.cur_col = cur.clamp(0, n as i64 - 1) as usize;
    }

    pub fn goto_row(&mut self, view_idx: u64) {
        let n = self.nrows();
        self.view.cur_row = view_idx.min(n.saturating_sub(1));
    }

    pub fn goto_first_row(&mut self) {
        self.view.cur_row = 0;
    }

    pub fn goto_last_row(&mut self) {
        self.view.cur_row = self.nrows().saturating_sub(1);
    }

    pub fn goto_first_col(&mut self) {
        self.view.cur_col = 0;
    }

    pub fn goto_last_col(&mut self) {
        self.view.cur_col = self.visible_cols().len().saturating_sub(1);
    }

    /// Keep cursor within data and scroll viewport to contain it.
    pub fn clamp_view(&mut self, screen_rows: u64) {
        let n = self.nrows();
        if self.view.cur_row >= n {
            self.view.cur_row = n.saturating_sub(1);
        }
        let ncols = self.visible_cols().len();
        if self.view.cur_col >= ncols {
            self.view.cur_col = ncols.saturating_sub(1);
        }
        if self.view.cur_row < self.view.top_row {
            self.view.top_row = self.view.cur_row;
        }
        if screen_rows > 0 && self.view.cur_row >= self.view.top_row + screen_rows {
            self.view.top_row = self.view.cur_row - screen_rows + 1;
        }
    }

    // ---- selection (RowId-keyed) ----

    pub fn toggle_select_cursor(&mut self) {
        if let Some(rid) = self.cursor_rowid() {
            if self.selection.contains(rid) {
                self.selection.remove(rid);
            } else {
                self.selection.insert(rid);
            }
        }
    }

    pub fn n_selected(&self) -> u64 {
        self.selection.len()
    }

    /// Iterate RowIds of all visible rows in view order.
    pub fn iter_rowids(&self) -> Box<dyn Iterator<Item = RowId> + '_> {
        match &self.perm {
            Some(p) => Box::new(p.iter().copied()),
            None => {
                let n = self.nrows();
                Box::new((0..n).filter_map(move |i| self.rowid_at(i)))
            }
        }
    }

    /// Selected RowIds in view order (or all rows if none selected) —
    /// visidata's "selected rows or whole sheet" convention for transforms.
    pub fn selected_or_all(&self) -> Vec<RowId> {
        if self.selection.is_empty() {
            self.iter_rowids().collect()
        } else {
            self.iter_rowids()
                .filter(|r| self.selection.contains(*r))
                .collect()
        }
    }

    /// A read-only clone for background jobs (sort/freq/pivot): shares the
    /// source Arc, snapshots the overlay, drops undo history.
    pub fn job_snapshot(&self) -> Sheet {
        Sheet {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind,
            source: self.source.clone(),
            overlay: self.overlay.snapshot(),
            columns: self.columns.clone(),
            perm: self.perm.clone(),
            selection: self.selection.clone(),
            sort_spec: self.sort_spec.clone(),
            view: self.view.clone(),
            undo: UndoStack::new(),
            parent: self.parent,
            groups: self.groups.clone(),
            source_path: self.source_path.clone(),
        }
    }

    /// Remove a RowId from the active permutation (after deletion).
    pub fn remove_from_perm(&mut self, rowid: RowId) {
        if let Some(p) = &mut self.perm {
            let v = Arc::make_mut(p);
            if let Some(pos) = v.iter().position(|&r| r == rowid) {
                v.remove(pos);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ColumnarStore;

    fn sheet3() -> Sheet {
        let store = ColumnarStore::from_rows(
            1,
            (0..10).map(|i| vec![Value::Int(i)]).collect::<Vec<_>>(),
        );
        let cols = vec![Column::source("n", 0)];
        Sheet::new("t", Arc::new(store), cols)
    }

    #[test]
    fn natural_order_mapping() {
        let s = sheet3();
        assert_eq!(s.nrows(), 10);
        assert_eq!(s.rowid_at(3), Some(3));
        assert_eq!(s.view_index_of(3), Some(3));
    }

    #[test]
    fn deleted_rows_skipped() {
        let mut s = sheet3();
        s.overlay.delete_row(0);
        s.overlay.delete_row(5);
        assert_eq!(s.nrows(), 8);
        assert_eq!(s.rowid_at(0), Some(1));
        assert_eq!(s.rowid_at(3), Some(4));
        assert_eq!(s.rowid_at(4), Some(6)); // skips 5
        assert_eq!(s.view_index_of(6), Some(4));
    }

    #[test]
    fn inserted_rows_at_end() {
        let mut s = sheet3();
        let id = s.overlay.insert_row(vec![Value::Int(99)]);
        assert_eq!(s.nrows(), 11);
        assert_eq!(s.rowid_at(10), Some(id));
        assert_eq!(s.typed_at(10, 0), Value::Int(99));
    }

    #[test]
    fn permutation_mapping() {
        let mut s = sheet3();
        s.perm = Some(Arc::new(vec![9, 8, 7]));
        assert_eq!(s.nrows(), 3);
        assert_eq!(s.rowid_at(0), Some(9));
        assert_eq!(s.view_index_of(7), Some(2));
    }

    #[test]
    fn cursor_clamping() {
        let mut s = sheet3();
        s.move_cursor_rows(100);
        assert_eq!(s.view.cur_row, 9);
        s.move_cursor_rows(-100);
        assert_eq!(s.view.cur_row, 0);
        s.goto_last_row();
        assert_eq!(s.view.cur_row, 9);
    }

    #[test]
    fn selection_survives_view() {
        let mut s = sheet3();
        s.view.cur_row = 2;
        s.toggle_select_cursor();
        assert!(s.selection.contains(2));
        assert_eq!(s.selected_or_all(), vec![2]);
    }

    #[test]
    fn key_cols_pinned_first() {
        let store = ColumnarStore::from_rows(3, vec![vec![Value::Int(1), Value::Int(2), Value::Int(3)]]);
        let mut cols = vec![
            Column::source("a", 0),
            Column::source("b", 1),
            Column::source("c", 2),
        ];
        cols[2].key_pos = Some(0);
        let s = Sheet::new("t", Arc::new(store), cols);
        assert_eq!(s.visible_cols(), vec![2, 0, 1]);
    }
}

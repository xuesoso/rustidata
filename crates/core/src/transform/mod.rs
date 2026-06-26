//! Derived-sheet transforms (visidata view-manipulation parity).
//!
//! Each transform reads a parent sheet and produces a new Sheet backed by an
//! in-memory ColumnarStore. Where meaningful, the result carries `groups`
//! (source RowIds per derived row) so Enter can drill back into the parent.

pub mod describe;
pub mod fill;
pub mod freeze;
pub mod freq;
pub mod join;
pub mod melt;
pub mod pivot;
pub mod row_detail;
pub mod transpose;
pub mod unfurl;

use std::sync::Arc;

use crate::column::Column;
use crate::sheet::{Sheet, SheetKind};
use crate::store::{ColumnarStore, RowId};
use crate::value::Value;

/// Build a derived sheet from realized rows of Values.
pub(crate) fn derived_sheet(
    name: String,
    kind: SheetKind,
    parent: &Sheet,
    col_names: Vec<String>,
    rows: Vec<Vec<Value>>,
    groups: Option<Vec<Vec<RowId>>>,
) -> Sheet {
    let ncols = col_names.len();
    let store = ColumnarStore::from_rows(ncols, rows);
    let columns = col_names
        .into_iter()
        .enumerate()
        .map(|(i, n)| Column::source(n, i))
        .collect();
    let mut sheet = Sheet::new(name, Arc::new(store), columns).with_kind(kind);
    sheet.parent = Some(parent.id);
    sheet.groups = groups.map(Arc::new);
    sheet
}

/// A sheet restricted to a set of parent RowIds (drill-down from freq/pivot).
/// Shares the parent's store; the subset is expressed as a permutation.
pub fn subset_sheet(parent: &Sheet, name: String, rowids: Vec<RowId>) -> Sheet {
    let mut sheet = Sheet::new(name, parent.source.clone(), parent.columns.clone());
    sheet.kind = parent.kind;
    sheet.parent = Some(parent.id);
    sheet.perm = Some(Arc::new(rowids));
    sheet.source_path = parent.source_path.clone();
    sheet
}

//! Fill (visidata `f`): replace nulls in a column with the previous non-null
//! value (in view order). Implemented as overlay edits so it is undoable.

use crate::sheet::Sheet;
use crate::undo::UndoOp;
use crate::value::Value;

/// Returns the undo ops performed (already applied to the sheet's overlay).
pub fn fill_forward(sheet: &mut Sheet, col: usize) -> Vec<UndoOp> {
    let rowids: Vec<_> = sheet.iter_rowids().collect();
    let mut last: Option<Value> = None;
    let mut ops = Vec::new();
    for rid in rowids {
        let v = sheet.columns[col].typed_value(sheet, rid);
        if v.is_null() {
            if let Some(fill) = &last {
                let src_idx = match &sheet.columns[col].kind {
                    crate::column::ColumnKind::Source(i) => *i,
                    _ => continue, // only source columns are fillable in place
                };
                let old = sheet.overlay.set_cell(rid, src_idx, fill.clone());
                ops.push(UndoOp::SetCell {
                    row: rid,
                    col: src_idx,
                    old,
                    new: fill.clone(),
                });
            }
        } else if !v.is_error() {
            last = Some(v);
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    #[test]
    fn fills_nulls_forward() {
        let store = ColumnarStore::from_rows(
            1,
            vec![
                vec![Value::Str("a".into())],
                vec![Value::Null],
                vec![Value::Null],
                vec![Value::Str("b".into())],
                vec![Value::Null],
            ],
        );
        let mut s = Sheet::new("t", Arc::new(store), vec![Column::source("c", 0)]);
        let ops = fill_forward(&mut s, 0);
        assert_eq!(ops.len(), 3);
        assert_eq!(s.typed_at(1, 0), Value::Str("a".into()));
        assert_eq!(s.typed_at(2, 0), Value::Str("a".into()));
        assert_eq!(s.typed_at(4, 0), Value::Str("b".into()));
    }
}

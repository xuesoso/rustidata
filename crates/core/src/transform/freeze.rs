//! Freeze (visidata `'` / `g'`): materialize computed/expression columns as
//! static values.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::column::{Column, ColumnKind};
use crate::sheet::Sheet;
use crate::value::Value;

/// Freeze one column in place: its current typed values are captured into a
/// Frozen column that replaces it.
pub fn freeze_column(sheet: &mut Sheet, col: usize) {
    let mut values: HashMap<crate::store::RowId, Value> = HashMap::new();
    let rowids: Vec<_> = sheet.iter_rowids().collect();
    for rid in rowids {
        values.insert(rid, sheet.columns[col].typed_value(sheet, rid));
    }
    let old = &sheet.columns[col];
    sheet.columns[col] = Column {
        name: old.name.clone(),
        kind: ColumnKind::Frozen(Arc::new(Mutex::new(values))),
        vtype: old.vtype,
        width: old.width,
        fmtstr: old.fmtstr.clone(),
        hidden: old.hidden,
        key_pos: old.key_pos,
    };
}

/// Freeze the whole sheet (visidata `g'`): a new static sheet from all
/// current visible values in view order.
pub fn freeze_sheet(parent: &Sheet) -> Sheet {
    let vis = parent.visible_cols();
    let col_names: Vec<String> = vis
        .iter()
        .map(|&c| parent.columns[c].name.clone())
        .collect();
    let rows: Vec<Vec<Value>> = parent
        .iter_rowids()
        .map(|rid| {
            vis.iter()
                .map(|&c| parent.columns[c].typed_value(parent, rid))
                .collect()
        })
        .collect();
    let mut sheet = super::derived_sheet(
        format!("{}_frozen", parent.name),
        parent.kind,
        parent,
        col_names,
        rows,
        None,
    );
    // preserve types and key columns
    for (out_i, &src_i) in vis.iter().enumerate() {
        sheet.columns[out_i].vtype = parent.columns[src_i].vtype;
        sheet.columns[out_i].key_pos = parent.columns[src_i].key_pos;
        sheet.columns[out_i].fmtstr = parent.columns[src_i].fmtstr.clone();
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ColumnarStore;

    #[test]
    fn freeze_column_captures_values() {
        let store = ColumnarStore::from_rows(
            1,
            vec![vec![Value::Int(1)], vec![Value::Int(2)]],
        );
        let mut s = Sheet::new("t", Arc::new(store), vec![Column::source("n", 0)]);
        freeze_column(&mut s, 0);
        assert!(matches!(s.columns[0].kind, ColumnKind::Frozen(_)));
        assert_eq!(s.typed_at(0, 0), Value::Int(1));
        assert_eq!(s.typed_at(1, 0), Value::Int(2));
    }

    #[test]
    fn freeze_sheet_snapshots() {
        let store = ColumnarStore::from_rows(
            1,
            vec![vec![Value::Int(5)], vec![Value::Int(6)]],
        );
        let p = Sheet::new("t", Arc::new(store), vec![Column::source("n", 0)]);
        let f = freeze_sheet(&p);
        assert_eq!(f.nrows(), 2);
        assert_eq!(f.typed_at(1, 0), Value::Int(6));
        assert_eq!(f.parent, Some(p.id));
    }
}

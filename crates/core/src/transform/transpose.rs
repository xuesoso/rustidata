//! Transpose (visidata `T`): columns become rows.

use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::value::Value;

pub fn transpose(parent: &Sheet, cancel: &CancelToken) -> Option<Sheet> {
    let rowids: Vec<_> = parent.iter_rowids().collect();
    let vis = parent.visible_cols();

    // output columns: "column" + one per source row (named by its first key
    // column value, or row index)
    let key_cols: Vec<usize> = (0..parent.columns.len())
        .filter(|&i| parent.columns[i].is_key())
        .collect();
    let mut col_names = vec!["column".to_string()];
    for (i, &rid) in rowids.iter().enumerate() {
        let name = match key_cols.first() {
            Some(&kc) => parent.columns[kc].typed_value(parent, rid).to_display().to_string(),
            None => format!("row{}", i),
        };
        col_names.push(name);
    }

    let mut rows = Vec::new();
    for &ci in &vis {
        if cancel.is_cancelled() {
            return None;
        }
        if key_cols.contains(&ci) {
            continue;
        }
        let mut row = vec![Value::Str(parent.columns[ci].name.as_str().into())];
        for &rid in &rowids {
            row.push(parent.columns[ci].typed_value(parent, rid));
        }
        rows.push(row);
    }

    let mut sheet = super::derived_sheet(
        format!("{}_T", parent.name),
        SheetKind::Transpose,
        parent,
        col_names,
        rows,
        None,
    );
    sheet.columns[0].key_pos = Some(0);
    Some(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    #[test]
    fn transpose_basic() {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Int(1), Value::Int(2)],
                vec![Value::Int(3), Value::Int(4)],
            ],
        );
        let p = Sheet::new(
            "t",
            Arc::new(store),
            vec![Column::source("a", 0), Column::source("b", 1)],
        );
        let s = transpose(&p, &CancelToken::new()).unwrap();
        assert_eq!(s.nrows(), 2);
        assert_eq!(s.columns.len(), 3); // column, row0, row1
        assert_eq!(s.typed_at(0, 0), Value::Str("a".into()));
        assert_eq!(s.typed_at(0, 1), Value::Int(1));
        assert_eq!(s.typed_at(1, 2), Value::Int(4));
    }
}

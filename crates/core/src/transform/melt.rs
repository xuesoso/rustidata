//! Melt (visidata `M`): unpivot non-key columns into (variable, value) rows.

use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::value::Value;

pub fn melt(parent: &Sheet, cancel: &CancelToken) -> Option<Sheet> {
    let key_cols: Vec<usize> = (0..parent.columns.len())
        .filter(|&i| parent.columns[i].is_key())
        .collect();
    let melt_cols: Vec<usize> = (0..parent.columns.len())
        .filter(|&i| !parent.columns[i].is_key() && !parent.columns[i].hidden)
        .collect();

    let mut col_names: Vec<String> = key_cols
        .iter()
        .map(|&c| parent.columns[c].name.clone())
        .collect();
    col_names.push("variable".into());
    col_names.push("value".into());

    let mut rows = Vec::new();
    let mut groups = Vec::new();
    for rid in parent.iter_rowids() {
        if cancel.is_cancelled() {
            return None;
        }
        let keyvals: Vec<Value> = key_cols
            .iter()
            .map(|&c| parent.columns[c].typed_value(parent, rid))
            .collect();
        for &mc in &melt_cols {
            let v = parent.columns[mc].typed_value(parent, rid);
            if v.is_null() {
                continue; // visidata skips nulls when melting
            }
            let mut row = keyvals.clone();
            row.push(Value::Str(parent.columns[mc].name.as_str().into()));
            row.push(v);
            rows.push(row);
            groups.push(vec![rid]);
        }
    }

    let mut sheet = super::derived_sheet(
        format!("{}_melted", parent.name),
        SheetKind::Melt,
        parent,
        col_names,
        rows,
        Some(groups),
    );
    for i in 0..key_cols.len() {
        sheet.columns[i].key_pos = Some(i as u8);
    }
    Some(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    #[test]
    fn melt_basic() {
        let store = ColumnarStore::from_rows(
            3,
            vec![
                vec![Value::Str("r1".into()), Value::Int(1), Value::Int(2)],
                vec![Value::Str("r2".into()), Value::Int(3), Value::Null],
            ],
        );
        let mut cols = vec![
            Column::source("id", 0),
            Column::source("x", 1),
            Column::source("y", 2),
        ];
        cols[0].key_pos = Some(0);
        let p = Sheet::new("t", Arc::new(store), cols);
        let m = melt(&p, &CancelToken::new()).unwrap();
        // r1/x/1, r1/y/2, r2/x/3 (null skipped)
        assert_eq!(m.nrows(), 3);
        assert_eq!(m.typed_at(0, 1), Value::Str("x".into()));
        assert_eq!(m.typed_at(0, 2), Value::Int(1));
        assert_eq!(m.typed_at(2, 0), Value::Str("r2".into()));
    }
}

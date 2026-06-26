//! Row detail (visidata's Enter on a row): expand one row into a sheet with a
//! `column` key and its `value`, one output row per visible source column.
//! Closing the sheet (`q`) returns to the parent table.

use crate::sheet::{Sheet, SheetKind};
use crate::store::RowId;
use crate::value::Value;

pub fn row_detail(parent: &Sheet, rid: RowId) -> Sheet {
    let vis = parent.visible_cols();
    let col_names = vec!["column".to_string(), "value".to_string()];
    let rows: Vec<Vec<Value>> = vis
        .iter()
        .map(|&ci| {
            vec![
                Value::Str(parent.columns[ci].name.as_str().into()),
                parent.columns[ci].typed_value(parent, rid),
            ]
        })
        .collect();

    // Label the sheet by the row's first key value, falling back to its id.
    let key_col = (0..parent.columns.len()).find(|&i| parent.columns[i].is_key());
    let label = match key_col {
        Some(kc) => parent.columns[kc].typed_value(parent, rid).to_display().to_string(),
        None => rid.to_string(),
    };

    let mut sheet = super::derived_sheet(
        format!("{}[{}]", parent.name, label),
        SheetKind::RowDetail,
        parent,
        col_names,
        rows,
        None,
    );
    sheet.columns[0].key_pos = Some(0);
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    #[test]
    fn row_detail_basic() {
        // from_rows is row-major: row 0 = (a=1, b="x"), row 1 = (a=2, b="y")
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Int(1), Value::Str("x".into())],
                vec![Value::Int(2), Value::Str("y".into())],
            ],
        );
        let p = Sheet::new(
            "t",
            Arc::new(store),
            vec![Column::source("a", 0), Column::source("b", 1)],
        );
        // detail of the first row: one output row per source column
        let s = row_detail(&p, 0);
        assert_eq!(s.columns.len(), 2); // column, value
        assert_eq!(s.nrows(), 2); // one per source column
        // typed_at(row, col): row 0 is column "a" -> 1; row 1 is "b" -> "x"
        assert_eq!(s.typed_at(0, 0), Value::Str("a".into()));
        assert_eq!(s.typed_at(0, 1), Value::Int(1));
        assert_eq!(s.typed_at(1, 0), Value::Str("b".into()));
        assert_eq!(s.typed_at(1, 1), Value::Str("x".into()));
        assert_eq!(s.kind, SheetKind::RowDetail);
    }
}

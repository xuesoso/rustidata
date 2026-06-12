//! Describe (visidata `I`): per-column summary statistics.

use rayon::prelude::*;

use crate::aggregate::Aggregator;
use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::value::Value;

pub fn describe(parent: &Sheet, cancel: &CancelToken) -> Option<Sheet> {
    let rowids: Vec<_> = parent.iter_rowids().collect();
    let vis = parent.visible_cols();

    let col_names: Vec<String> = vec![
        "column".into(),
        "type".into(),
        "count".into(),
        "nulls".into(),
        "distinct".into(),
        "mode".into(),
        "min".into(),
        "max".into(),
        "sum".into(),
        "mean".into(),
        "median".into(),
        "stdev".into(),
    ];

    let rows: Vec<Vec<Value>> = vis
        .par_iter()
        .map(|&ci| {
            let col = &parent.columns[ci];
            let values: Vec<Value> = rowids
                .iter()
                .map(|&rid| col.typed_value(parent, rid))
                .collect();
            let nulls = values.iter().filter(|v| v.is_null()).count();
            let numericish = col.vtype.is_numeric()
                || values
                    .iter()
                    .filter(|v| !v.is_null() && !v.is_error())
                    .all(|v| matches!(v, Value::Int(_) | Value::Float(_) | Value::Date(_)));
            let numeric_stat = |agg: Aggregator| {
                if numericish {
                    agg.apply(&values)
                } else {
                    Value::Null
                }
            };
            vec![
                Value::Str(col.name.as_str().into()),
                Value::Str(col.vtype.name().into()),
                Aggregator::Count.apply(&values),
                Value::Int(nulls as i64),
                Aggregator::Distinct.apply(&values),
                Aggregator::Mode.apply(&values),
                Aggregator::Min.apply(&values),
                Aggregator::Max.apply(&values),
                numeric_stat(Aggregator::Sum),
                numeric_stat(Aggregator::Avg),
                numeric_stat(Aggregator::Median),
                numeric_stat(Aggregator::Stdev),
            ]
        })
        .collect();

    if cancel.is_cancelled() {
        return None;
    }

    let mut sheet = super::derived_sheet(
        format!("{}_describe", parent.name),
        SheetKind::Describe,
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
    use crate::value::VType;
    use std::sync::Arc;

    #[test]
    fn describe_stats() {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("x".into()), Value::Int(1)],
                vec![Value::Str("y".into()), Value::Int(3)],
                vec![Value::Null, Value::Int(2)],
            ],
        );
        let mut cols = vec![Column::source("name", 0), Column::source("n", 1)];
        cols[1].vtype = VType::Int;
        let p = Sheet::new("t", Arc::new(store), cols);
        let d = describe(&p, &CancelToken::new()).unwrap();
        assert_eq!(d.nrows(), 2);
        // row 1 = column "n"
        assert_eq!(d.typed_at(1, 0), Value::Str("n".into()));
        assert_eq!(d.typed_at(1, 2), Value::Int(3)); // count
        assert_eq!(d.typed_at(1, 3), Value::Int(0)); // nulls
        assert_eq!(d.typed_at(1, 8), Value::Int(6)); // sum
        assert_eq!(d.typed_at(1, 9), Value::Float(2.0)); // mean
        // row 0 = column "name": no numeric stats
        assert_eq!(d.typed_at(0, 3), Value::Int(1)); // one null
        assert_eq!(d.typed_at(0, 8), Value::Null);
    }
}

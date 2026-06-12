//! Pivot (visidata `W`): rows grouped by key columns, one output column per
//! distinct value of the pivot column, cells aggregated from the value column.

use std::collections::HashMap;

use compact_str::CompactString;

use crate::aggregate::Aggregator;
use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::store::RowId;
use crate::value::Value;

pub fn pivot(
    parent: &Sheet,
    key_cols: &[usize],
    pivot_col: usize,
    value_col: usize,
    agg: Aggregator,
    cancel: &CancelToken,
) -> Option<Sheet> {
    // distinct pivot values in first-seen order
    let mut pivot_vals: Vec<CompactString> = Vec::new();
    let mut pivot_idx: HashMap<CompactString, usize> = HashMap::new();
    // group key -> (key values, per-pivot-value Vec<Value>, all rowids)
    let mut groups: Vec<(Vec<Value>, Vec<Vec<Value>>, Vec<RowId>)> = Vec::new();
    let mut group_idx: HashMap<Vec<CompactString>, usize> = HashMap::new();

    for rid in parent.iter_rowids() {
        if cancel.is_cancelled() {
            return None;
        }
        let pv = parent.columns[pivot_col]
            .typed_value(parent, rid)
            .to_display();
        let pi = *pivot_idx.entry(pv.clone()).or_insert_with(|| {
            pivot_vals.push(pv.clone());
            pivot_vals.len() - 1
        });

        let keyvals: Vec<Value> = key_cols
            .iter()
            .map(|&c| parent.columns[c].typed_value(parent, rid))
            .collect();
        let key: Vec<CompactString> = keyvals.iter().map(|v| v.to_display()).collect();
        let gi = match group_idx.get(&key) {
            Some(&gi) => gi,
            None => {
                group_idx.insert(key, groups.len());
                groups.push((keyvals, Vec::new(), Vec::new()));
                groups.len() - 1
            }
        };
        let g = &mut groups[gi];
        if g.1.len() <= pi {
            g.1.resize_with(pi + 1, Vec::new);
        }
        g.1[pi].push(parent.columns[value_col].typed_value(parent, rid));
        g.2.push(rid);
    }

    let mut col_names: Vec<String> = key_cols
        .iter()
        .map(|&c| parent.columns[c].name.clone())
        .collect();
    for pv in &pivot_vals {
        col_names.push(pv.to_string());
    }

    let mut rows = Vec::with_capacity(groups.len());
    let mut group_ids = Vec::with_capacity(groups.len());
    for (keyvals, cells, rids) in groups {
        let mut row = keyvals;
        for pi in 0..pivot_vals.len() {
            let vals = cells.get(pi).map(|v| v.as_slice()).unwrap_or(&[]);
            row.push(if vals.is_empty() {
                Value::Null
            } else {
                agg.apply(vals)
            });
        }
        rows.push(row);
        group_ids.push(rids);
    }

    let mut sheet = super::derived_sheet(
        format!(
            "{}_{}_pivot",
            parent.name, parent.columns[pivot_col].name
        ),
        SheetKind::Pivot,
        parent,
        col_names,
        rows,
        Some(group_ids),
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
    use crate::value::VType;
    use std::sync::Arc;

    fn parent() -> Sheet {
        // city, year, sales
        let store = ColumnarStore::from_rows(
            3,
            vec![
                vec![Value::Str("nyc".into()), Value::Str("2023".into()), Value::Int(10)],
                vec![Value::Str("nyc".into()), Value::Str("2024".into()), Value::Int(20)],
                vec![Value::Str("sf".into()), Value::Str("2023".into()), Value::Int(5)],
                vec![Value::Str("nyc".into()), Value::Str("2023".into()), Value::Int(7)],
            ],
        );
        let mut cols = vec![
            Column::source("city", 0),
            Column::source("year", 1),
            Column::source("sales", 2),
        ];
        cols[2].vtype = VType::Int;
        Sheet::new("t", Arc::new(store), cols)
    }

    #[test]
    fn pivot_sum() {
        let p = parent();
        let s = pivot(&p, &[0], 1, 2, Aggregator::Sum, &CancelToken::new()).unwrap();
        // columns: city, 2023, 2024
        assert_eq!(s.columns.len(), 3);
        assert_eq!(s.nrows(), 2);
        // nyc: 2023 -> 17, 2024 -> 20
        assert_eq!(s.typed_at(0, 0), Value::Str("nyc".into()));
        assert_eq!(s.typed_at(0, 1), Value::Int(17));
        assert_eq!(s.typed_at(0, 2), Value::Int(20));
        // sf: 2023 -> 5, 2024 -> null
        assert_eq!(s.typed_at(1, 1), Value::Int(5));
        assert_eq!(s.typed_at(1, 2), Value::Null);
    }
}

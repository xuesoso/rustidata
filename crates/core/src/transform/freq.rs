//! Frequency table (visidata `F`): group by one or more columns, count +
//! percent + histogram, plus any requested aggregations of other columns.

use std::collections::HashMap;

use compact_str::CompactString;

use crate::aggregate::Aggregator;
use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::store::RowId;
use crate::value::Value;

/// An aggregation request: apply `agg` to `col` per group.
#[derive(Clone, Copy)]
pub struct AggSpec {
    pub col: usize,
    pub agg: Aggregator,
}

pub fn freq_table(
    parent: &Sheet,
    group_cols: &[usize],
    aggs: &[AggSpec],
    cancel: &CancelToken,
) -> Option<Sheet> {
    let mut groups: Vec<(Vec<Value>, Vec<RowId>)> = Vec::new();
    let mut index: HashMap<Vec<CompactString>, usize> = HashMap::new();

    for rid in parent.iter_rowids() {
        if cancel.is_cancelled() {
            return None;
        }
        let typed: Vec<Value> = group_cols
            .iter()
            .map(|&c| parent.columns[c].typed_value(parent, rid))
            .collect();
        let key: Vec<CompactString> = typed.iter().map(|v| v.to_display()).collect();
        match index.get(&key) {
            Some(&gi) => groups[gi].1.push(rid),
            None => {
                index.insert(key, groups.len());
                groups.push((typed, vec![rid]));
            }
        }
    }

    let total: u64 = groups.iter().map(|(_, ids)| ids.len() as u64).sum();
    let max_count = groups.iter().map(|(_, ids)| ids.len()).max().unwrap_or(1);

    // sort by count descending, like visidata
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut col_names: Vec<String> = group_cols
        .iter()
        .map(|&c| parent.columns[c].name.clone())
        .collect();
    col_names.push("count".into());
    col_names.push("percent".into());
    col_names.push("histogram".into());
    for a in aggs {
        col_names.push(format!("{}_{}", parent.columns[a.col].name, a.agg.name()));
    }

    let mut rows = Vec::with_capacity(groups.len());
    let mut group_ids = Vec::with_capacity(groups.len());
    for (keyvals, rids) in &groups {
        if cancel.is_cancelled() {
            return None;
        }
        let count = rids.len();
        let mut row: Vec<Value> = keyvals.clone();
        row.push(Value::Int(count as i64));
        row.push(Value::Float(
            if total > 0 {
                count as f64 * 100.0 / total as f64
            } else {
                0.0
            },
        ));
        row.push(Value::Str(histogram_bar(count, max_count, 40)));
        for a in aggs {
            let vals: Vec<Value> = rids
                .iter()
                .map(|&r| parent.columns[a.col].typed_value(parent, r))
                .collect();
            row.push(a.agg.apply(&vals));
        }
        rows.push(row);
        group_ids.push(rids.clone());
    }

    let names = group_cols
        .iter()
        .map(|&c| parent.columns[c].name.as_str())
        .collect::<Vec<_>>()
        .join("-");
    let mut sheet = super::derived_sheet(
        format!("{}_{}_freq", parent.name, names),
        SheetKind::Freq,
        parent,
        col_names,
        rows,
        Some(group_ids),
    );
    // group-key columns become key columns
    for i in 0..group_cols.len() {
        sheet.columns[i].key_pos = Some(i as u8);
    }
    sheet.columns[group_cols.len()].vtype = crate::value::VType::Int; // count
    sheet.columns[group_cols.len() + 1].vtype = crate::value::VType::Float; // percent
    sheet.columns[group_cols.len() + 1].fmtstr = Some("{:.2f}".into());
    Some(sheet)
}

fn histogram_bar(count: usize, max: usize, width: usize) -> CompactString {
    let n = if max == 0 {
        0
    } else {
        (count * width + max - 1) / max
    };
    "■".repeat(n.min(width)).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    fn parent() -> Sheet {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("a".into()), Value::Int(10)],
                vec![Value::Str("b".into()), Value::Int(20)],
                vec![Value::Str("a".into()), Value::Int(30)],
                vec![Value::Str("a".into()), Value::Int(40)],
            ],
        );
        let mut cols = vec![Column::source("k", 0), Column::source("v", 1)];
        cols[1].vtype = crate::value::VType::Int;
        Sheet::new("t", Arc::new(store), cols)
    }

    #[test]
    fn counts_and_percent() {
        let p = parent();
        let f = freq_table(&p, &[0], &[], &CancelToken::new()).unwrap();
        assert_eq!(f.nrows(), 2);
        // sorted by count desc: a=3, b=1
        assert_eq!(f.typed_at(0, 0), Value::Str("a".into()));
        assert_eq!(f.typed_at(0, 1), Value::Int(3));
        assert_eq!(f.typed_at(1, 1), Value::Int(1));
        if let Value::Float(pct) = f.typed_at(0, 2) {
            assert!((pct - 75.0).abs() < 1e-9);
        } else {
            panic!("expected percent float");
        }
    }

    #[test]
    fn aggregation() {
        let p = parent();
        let f = freq_table(
            &p,
            &[0],
            &[AggSpec {
                col: 1,
                agg: Aggregator::Sum,
            }],
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(f.typed_at(0, 4), Value::Int(80)); // 10+30+40
    }

    #[test]
    fn groups_back_reference() {
        let p = parent();
        let f = freq_table(&p, &[0], &[], &CancelToken::new()).unwrap();
        let groups = f.groups.as_ref().unwrap();
        assert_eq!(groups[0], vec![0, 2, 3]);
        assert_eq!(groups[1], vec![1]);
    }

    #[test]
    fn respects_selection_ordering_via_iter() {
        let p = parent();
        let f = freq_table(&p, &[0], &[], &CancelToken::new()).unwrap();
        assert!(f.columns[0].is_key());
    }
}

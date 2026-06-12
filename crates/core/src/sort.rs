//! Parallel sort producing a RowId permutation (data never moves).

use std::sync::Arc;

use rayon::prelude::*;

use crate::jobs::CancelToken;
use crate::sheet::Sheet;
use crate::store::RowId;
use crate::value::{value_cmp, Value};

/// Compute the sorted permutation of all visible rows for a sort spec
/// `[(columns index, descending), ...]`. Runs on the caller's thread (wrap in
/// a job for big sheets); checks `cancel` between phases.
pub fn sort_permutation(
    sheet: &Sheet,
    spec: &[(usize, bool)],
    cancel: &CancelToken,
) -> Option<Arc<Vec<RowId>>> {
    let ids: Vec<RowId> = sheet.iter_rowids().collect();
    if cancel.is_cancelled() {
        return None;
    }

    // Materialize sort keys once per row (typed values), in parallel.
    let keys: Vec<Vec<Value>> = ids
        .par_iter()
        .map(|&rid| {
            spec.iter()
                .map(|&(col, _)| sheet.columns[col].typed_value(sheet, rid))
                .collect()
        })
        .collect();
    if cancel.is_cancelled() {
        return None;
    }

    let mut order: Vec<u32> = (0..ids.len() as u32).collect();
    order.par_sort_by(|&a, &b| {
        let ka = &keys[a as usize];
        let kb = &keys[b as usize];
        for (i, &(_, desc)) in spec.iter().enumerate() {
            let ord = value_cmp(&ka[i], &kb[i]);
            let ord = if desc { ord.reverse() } else { ord };
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    });
    if cancel.is_cancelled() {
        return None;
    }

    Some(Arc::new(
        order.into_iter().map(|i| ids[i as usize]).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use crate::value::VType;

    fn sheet() -> Sheet {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("b".into()), Value::Int(2)],
                vec![Value::Str("a".into()), Value::Int(3)],
                vec![Value::Str("a".into()), Value::Int(1)],
            ],
        );
        let cols = vec![
            Column::source("k", 0),
            Column::source("v", 1).with_type(VType::Int),
        ];
        Sheet::new("t", Arc::new(store), cols)
    }

    #[test]
    fn single_key_asc() {
        let s = sheet();
        let p = sort_permutation(&s, &[(1, false)], &CancelToken::new()).unwrap();
        assert_eq!(&*p, &vec![2, 0, 1]);
    }

    #[test]
    fn multi_key() {
        let s = sheet();
        let p = sort_permutation(&s, &[(0, false), (1, true)], &CancelToken::new()).unwrap();
        // a/3, a/1, b/2
        assert_eq!(&*p, &vec![1, 2, 0]);
    }

    #[test]
    fn descending() {
        let s = sheet();
        let p = sort_permutation(&s, &[(1, true)], &CancelToken::new()).unwrap();
        assert_eq!(&*p, &vec![1, 0, 2]);
    }
}

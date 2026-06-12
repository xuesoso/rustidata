//! Joins (visidata `&` on the sheets sheet): inner/outer/full/diff/append/
//! extend/merge, keyed on each sheet's key columns.

use std::collections::HashMap;

use compact_str::CompactString;

use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// Rows with keys present in all sheets.
    Inner,
    /// All rows from the first sheet, matched where possible.
    Outer,
    /// All rows from all sheets.
    Full,
    /// Rows in the first sheet whose key is NOT in any later sheet.
    Diff,
    /// Concatenate rows; columns unioned by name.
    Append,
    /// Like outer, but fills missing cells in the first sheet from later ones.
    Extend,
    /// Like full, but later sheets' values overwrite earlier ones per cell.
    Merge,
}

impl JoinKind {
    pub fn from_name(s: &str) -> Option<JoinKind> {
        Some(match s {
            "inner" => JoinKind::Inner,
            "outer" => JoinKind::Outer,
            "full" => JoinKind::Full,
            "diff" => JoinKind::Diff,
            "append" => JoinKind::Append,
            "extend" => JoinKind::Extend,
            "merge" => JoinKind::Merge,
            _ => return None,
        })
    }

    pub fn all_names() -> &'static [&'static str] {
        &["inner", "outer", "full", "diff", "append", "extend", "merge"]
    }
}

fn key_of(sheet: &Sheet, rid: crate::store::RowId, key_cols: &[usize]) -> Vec<CompactString> {
    key_cols
        .iter()
        .map(|&c| sheet.columns[c].typed_value(sheet, rid).to_display())
        .collect()
}

fn key_cols_of(sheet: &Sheet) -> Vec<usize> {
    let mut keys: Vec<usize> = (0..sheet.columns.len())
        .filter(|&i| sheet.columns[i].is_key())
        .collect();
    keys.sort_by_key(|&i| sheet.columns[i].key_pos);
    keys
}

/// Join `sheets` (2+). All sheets must have the same number of key columns
/// (except Append, which needs none).
pub fn join(sheets: &[&Sheet], kind: JoinKind, cancel: &CancelToken) -> Result<Sheet, String> {
    if sheets.len() < 2 {
        return Err("join requires at least 2 sheets".into());
    }
    if kind == JoinKind::Append {
        return append_sheets(sheets, cancel);
    }

    let first = sheets[0];
    let key_cols0 = key_cols_of(first);
    if key_cols0.is_empty() {
        return Err("set key column(s) with ! before joining".into());
    }
    for s in sheets {
        if key_cols_of(s).len() != key_cols0.len() {
            return Err(format!(
                "sheet {} has a different number of key columns",
                s.name
            ));
        }
    }

    // column layout: key cols, then each sheet's non-key visible columns
    let mut col_names: Vec<String> = key_cols0
        .iter()
        .map(|&c| first.columns[c].name.clone())
        .collect();
    // (sheet idx, column idx) for each output value column
    let mut value_cols: Vec<(usize, usize)> = Vec::new();
    // merge dedups columns by name
    let mut merge_name_to_out: HashMap<String, usize> = HashMap::new();
    for (si, s) in sheets.iter().enumerate() {
        let kcols = key_cols_of(s);
        for ci in 0..s.columns.len() {
            if kcols.contains(&ci) || s.columns[ci].hidden {
                continue;
            }
            let name = &s.columns[ci].name;
            if kind == JoinKind::Merge || kind == JoinKind::Extend {
                if merge_name_to_out.contains_key(name) {
                    value_cols.push((si, ci)); // still consulted for fill
                    continue;
                }
                merge_name_to_out.insert(name.clone(), col_names.len());
            }
            value_cols.push((si, ci));
            col_names.push(if sheets.len() > 2 || kind == JoinKind::Merge || kind == JoinKind::Extend {
                name.clone()
            } else if si == 0 {
                name.clone()
            } else {
                format!("{}_{}", s.name, name)
            });
        }
    }

    // index every sheet by key
    let mut maps: Vec<HashMap<Vec<CompactString>, Vec<crate::store::RowId>>> = Vec::new();
    let mut key_orders: Vec<Vec<Vec<CompactString>>> = Vec::new();
    for s in sheets {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let kcols = key_cols_of(s);
        let mut m: HashMap<Vec<CompactString>, Vec<crate::store::RowId>> = HashMap::new();
        let mut order = Vec::new();
        for rid in s.iter_rowids() {
            let k = key_of(s, rid, &kcols);
            if !m.contains_key(&k) {
                order.push(k.clone());
            }
            m.entry(k).or_default().push(rid);
        }
        maps.push(m);
        key_orders.push(order);
    }

    // decide which keys appear in the output, in deterministic order
    let mut out_keys: Vec<Vec<CompactString>> = Vec::new();
    match kind {
        JoinKind::Inner => {
            for k in &key_orders[0] {
                if maps.iter().all(|m| m.contains_key(k)) {
                    out_keys.push(k.clone());
                }
            }
        }
        JoinKind::Outer | JoinKind::Extend => {
            out_keys = key_orders[0].clone();
        }
        JoinKind::Full | JoinKind::Merge => {
            let mut seen = std::collections::HashSet::new();
            for order in &key_orders {
                for k in order {
                    if seen.insert(k.clone()) {
                        out_keys.push(k.clone());
                    }
                }
            }
        }
        JoinKind::Diff => {
            for k in &key_orders[0] {
                if maps[1..].iter().all(|m| !m.contains_key(k)) {
                    out_keys.push(k.clone());
                }
            }
        }
        JoinKind::Append => unreachable!(),
    }

    let mut rows = Vec::with_capacity(out_keys.len());
    let mut groups = Vec::with_capacity(out_keys.len());
    for k in &out_keys {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let mut row: Vec<Value> = k
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Value::Null
                } else {
                    Value::Str(s.clone())
                }
            })
            .collect();
        // realized cells per output column name for merge/extend dedup
        let mut by_out_idx: HashMap<usize, Value> = HashMap::new();
        let mut grp: Vec<crate::store::RowId> = Vec::new();

        let mut out_idx = key_cols0.len();
        let mut merge_seen: HashMap<String, usize> = HashMap::new();
        for &(si, ci) in &value_cols {
            let s = sheets[si];
            let rid = maps[si].get(k).and_then(|v| v.first()).copied();
            if si == 0 {
                if let Some(r) = rid {
                    grp.push(r);
                }
            }
            let val = match rid {
                Some(r) => s.columns[ci].typed_value(s, r),
                None => Value::Null,
            };
            if kind == JoinKind::Merge || kind == JoinKind::Extend {
                let name = s.columns[ci].name.clone();
                let oi = *merge_seen
                    .entry(name.clone())
                    .or_insert_with(|| *merge_name_to_out.get(&name).unwrap_or(&out_idx));
                let entry = by_out_idx.entry(oi).or_insert(Value::Null);
                let should_overwrite = match kind {
                    JoinKind::Merge => !val.is_null(),
                    _ => entry.is_null() && !val.is_null(), // extend fills gaps only
                };
                if should_overwrite {
                    *entry = val;
                }
                if merge_name_to_out.get(&name) == Some(&oi) && oi >= out_idx {
                    out_idx = oi + 1;
                }
            } else {
                by_out_idx.insert(out_idx, val);
                out_idx += 1;
            }
        }
        let total_cols = col_names.len();
        for oi in key_cols0.len()..total_cols {
            row.push(by_out_idx.remove(&oi).unwrap_or(Value::Null));
        }
        rows.push(row);
        groups.push(grp);
    }

    let name = sheets
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("+");
    let mut sheet = super::derived_sheet(
        name,
        SheetKind::Join,
        first,
        col_names,
        rows,
        Some(groups),
    );
    for i in 0..key_cols0.len() {
        sheet.columns[i].key_pos = Some(i as u8);
    }
    Ok(sheet)
}

fn append_sheets(sheets: &[&Sheet], cancel: &CancelToken) -> Result<Sheet, String> {
    // union of column names in first-seen order
    let mut col_names: Vec<String> = Vec::new();
    let mut name_idx: HashMap<String, usize> = HashMap::new();
    for s in sheets {
        for c in &s.columns {
            if c.hidden {
                continue;
            }
            if !name_idx.contains_key(&c.name) {
                name_idx.insert(c.name.clone(), col_names.len());
                col_names.push(c.name.clone());
            }
        }
    }
    let mut rows = Vec::new();
    for s in sheets {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        for rid in s.iter_rowids() {
            let mut row = vec![Value::Null; col_names.len()];
            for (ci, c) in s.columns.iter().enumerate() {
                if c.hidden {
                    continue;
                }
                if let Some(&oi) = name_idx.get(&c.name) {
                    row[oi] = s.columns[ci].typed_value(s, rid);
                }
            }
            rows.push(row);
        }
    }
    let name = sheets
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("+");
    Ok(super::derived_sheet(
        name,
        SheetKind::Join,
        sheets[0],
        col_names,
        rows,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    fn sheet_a() -> Sheet {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("k1".into()), Value::Int(1)],
                vec![Value::Str("k2".into()), Value::Int(2)],
            ],
        );
        let mut cols = vec![Column::source("id", 0), Column::source("a", 1)];
        cols[0].key_pos = Some(0);
        let mut s = Sheet::new("A", Arc::new(store), cols);
        s.kind = SheetKind::Table;
        s
    }

    fn sheet_b() -> Sheet {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("k2".into()), Value::Int(20)],
                vec![Value::Str("k3".into()), Value::Int(30)],
            ],
        );
        let mut cols = vec![Column::source("id", 0), Column::source("b", 1)];
        cols[0].key_pos = Some(0);
        Sheet::new("B", Arc::new(store), cols)
    }

    #[test]
    fn inner_join() {
        let (a, b) = (sheet_a(), sheet_b());
        let j = join(&[&a, &b], JoinKind::Inner, &CancelToken::new()).unwrap();
        assert_eq!(j.nrows(), 1);
        assert_eq!(j.typed_at(0, 0), Value::Str("k2".into()));
        assert_eq!(j.typed_at(0, 1), Value::Int(2));
        assert_eq!(j.typed_at(0, 2), Value::Int(20));
    }

    #[test]
    fn outer_join() {
        let (a, b) = (sheet_a(), sheet_b());
        let j = join(&[&a, &b], JoinKind::Outer, &CancelToken::new()).unwrap();
        assert_eq!(j.nrows(), 2);
        assert_eq!(j.typed_at(0, 2), Value::Null); // k1 has no B row
    }

    #[test]
    fn full_join() {
        let (a, b) = (sheet_a(), sheet_b());
        let j = join(&[&a, &b], JoinKind::Full, &CancelToken::new()).unwrap();
        assert_eq!(j.nrows(), 3);
    }

    #[test]
    fn diff_join() {
        let (a, b) = (sheet_a(), sheet_b());
        let j = join(&[&a, &b], JoinKind::Diff, &CancelToken::new()).unwrap();
        assert_eq!(j.nrows(), 1);
        assert_eq!(j.typed_at(0, 0), Value::Str("k1".into()));
    }

    #[test]
    fn append_join() {
        let (a, b) = (sheet_a(), sheet_b());
        let j = join(&[&a, &b], JoinKind::Append, &CancelToken::new()).unwrap();
        assert_eq!(j.nrows(), 4);
        // columns: id, a, b
        assert_eq!(j.columns.len(), 3);
        assert_eq!(j.typed_at(3, 0), Value::Str("k3".into()));
        assert_eq!(j.typed_at(3, 1), Value::Null);
    }

    #[test]
    fn requires_keys() {
        let store = ColumnarStore::from_rows(1, vec![vec![Value::Int(1)]]);
        let a = Sheet::new("X", Arc::new(store), vec![Column::source("x", 0)]);
        let b = sheet_b();
        assert!(join(&[&a, &b], JoinKind::Inner, &CancelToken::new()).is_err());
    }
}

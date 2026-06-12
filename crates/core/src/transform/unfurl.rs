//! Unfurl (visidata `zM`): expand a column whose cells contain lists
//! (JSON arrays or delimited strings) into one row per element.

use crate::jobs::CancelToken;
use crate::sheet::{Sheet, SheetKind};
use crate::value::Value;

/// Split a cell into elements. JSON-style arrays are split on elements;
/// otherwise we split strings on `;` or `,`. Scalars unfurl to themselves.
fn elements(v: &Value) -> Vec<Value> {
    match v {
        Value::Str(s) => {
            let t = s.trim();
            if t.starts_with('[') && t.ends_with(']') {
                // lightweight JSON array split
                let inner = &t[1..t.len() - 1];
                return split_top_level(inner)
                    .into_iter()
                    .map(|e| Value::Str(e.trim().trim_matches('"').into()))
                    .collect();
            }
            if t.contains(';') {
                return t.split(';').map(|e| Value::Str(e.trim().into())).collect();
            }
            if t.contains(',') {
                return t.split(',').map(|e| Value::Str(e.trim().into())).collect();
            }
            vec![v.clone()]
        }
        _ => vec![v.clone()],
    }
}

fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '[' | '{' if !in_str => depth += 1,
            ']' | '}' if !in_str => depth = depth.saturating_sub(1),
            ',' if !in_str && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

pub fn unfurl(parent: &Sheet, col: usize, cancel: &CancelToken) -> Option<Sheet> {
    let col_names: Vec<String> = parent
        .visible_cols()
        .iter()
        .map(|&c| parent.columns[c].name.clone())
        .collect();
    let vis = parent.visible_cols();
    let unfurl_pos = vis.iter().position(|&c| c == col)?;

    let mut rows = Vec::new();
    let mut groups = Vec::new();
    for rid in parent.iter_rowids() {
        if cancel.is_cancelled() {
            return None;
        }
        let base: Vec<Value> = vis
            .iter()
            .map(|&c| parent.columns[c].typed_value(parent, rid))
            .collect();
        for elem in elements(&base[unfurl_pos]) {
            let mut row = base.clone();
            row[unfurl_pos] = elem;
            rows.push(row);
            groups.push(vec![rid]);
        }
    }

    Some(super::derived_sheet(
        format!("{}_{}_unfurled", parent.name, parent.columns[col].name),
        SheetKind::Unfurl,
        parent,
        col_names,
        rows,
        Some(groups),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use std::sync::Arc;

    #[test]
    fn unfurl_semicolons() {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("r1".into()), Value::Str("a; b".into())],
                vec![Value::Str("r2".into()), Value::Str("c".into())],
            ],
        );
        let p = Sheet::new(
            "t",
            Arc::new(store),
            vec![Column::source("id", 0), Column::source("tags", 1)],
        );
        let s = unfurl(&p, 1, &CancelToken::new()).unwrap();
        assert_eq!(s.nrows(), 3);
        assert_eq!(s.typed_at(0, 1), Value::Str("a".into()));
        assert_eq!(s.typed_at(1, 1), Value::Str("b".into()));
        assert_eq!(s.typed_at(2, 0), Value::Str("r2".into()));
    }

    #[test]
    fn unfurl_json_array() {
        let store = ColumnarStore::from_rows(
            1,
            vec![vec![Value::Str("[1, 2, 3]".into())]],
        );
        let p = Sheet::new("t", Arc::new(store), vec![Column::source("xs", 0)]);
        let s = unfurl(&p, 0, &CancelToken::new()).unwrap();
        assert_eq!(s.nrows(), 3);
        assert_eq!(s.typed_at(2, 0), Value::Str("3".into()));
    }
}

//! Regex search over rows, forward and backward, from the cursor.

use regex::Regex;

use crate::jobs::CancelToken;
use crate::sheet::Sheet;

/// Rows scanned between cancellation checks.
const CANCEL_STRIDE: u64 = 4096;

/// Where to search: the cursor column or all visible columns.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    CursorCol,
    AllCols,
}

/// Find the next view index (after `from`, wrapping) whose row matches.
/// Checks `cancel` periodically so a scan over a huge sheet can be aborted
/// (a cancelled search returns None).
pub fn search(
    sheet: &Sheet,
    re: &Regex,
    scope: SearchScope,
    from: u64,
    backward: bool,
    cancel: &CancelToken,
) -> Option<u64> {
    let n = sheet.nrows();
    if n == 0 {
        return None;
    }
    let cols: Vec<usize> = match scope {
        SearchScope::CursorCol => sheet.cursor_col_index().into_iter().collect(),
        SearchScope::AllCols => sheet.visible_cols(),
    };
    let step_iter: Box<dyn Iterator<Item = u64>> = if backward {
        Box::new((1..n).map(move |i| (from + n - i) % n))
    } else {
        Box::new((1..n).map(move |i| (from + i) % n))
    };
    for (scanned, vi) in step_iter.enumerate() {
        if scanned as u64 % CANCEL_STRIDE == 0 && cancel.is_cancelled() {
            return None;
        }
        if row_matches(sheet, vi, &cols, re) {
            return Some(vi);
        }
    }
    // single-row match on the current row still counts
    if row_matches(sheet, from, &cols, re) {
        return Some(from);
    }
    None
}

fn row_matches(sheet: &Sheet, view_idx: u64, cols: &[usize], re: &Regex) -> bool {
    let Some(rid) = sheet.rowid_at(view_idx) else {
        return false;
    };
    cols.iter().any(|&c| {
        let disp = sheet.columns[c].display_value(sheet, rid);
        re.is_match(&disp)
    })
}

/// All matching RowIds (used by select-by-regex).
pub fn matching_rowids(sheet: &Sheet, re: &Regex, scope: SearchScope) -> Vec<u64> {
    let cols: Vec<usize> = match scope {
        SearchScope::CursorCol => sheet.cursor_col_index().into_iter().collect(),
        SearchScope::AllCols => sheet.visible_cols(),
    };
    let mut out = Vec::new();
    for vi in 0..sheet.nrows() {
        if let Some(rid) = sheet.rowid_at(vi) {
            if cols.iter().any(|&c| {
                let disp = sheet.columns[c].display_value(sheet, rid);
                re.is_match(&disp)
            }) {
                out.push(rid);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::store::ColumnarStore;
    use crate::value::Value;
    use std::sync::Arc;

    fn sheet() -> Sheet {
        let store = ColumnarStore::from_rows(
            1,
            vec![
                vec![Value::Str("apple".into())],
                vec![Value::Str("banana".into())],
                vec![Value::Str("cherry".into())],
                vec![Value::Str("apricot".into())],
            ],
        );
        Sheet::new("t", Arc::new(store), vec![Column::source("f", 0)])
    }

    #[test]
    fn forward_wrap() {
        let s = sheet();
        let re = Regex::new("^ap").unwrap();
        let c = CancelToken::new();
        assert_eq!(search(&s, &re, SearchScope::AllCols, 0, false, &c), Some(3));
        assert_eq!(search(&s, &re, SearchScope::AllCols, 3, false, &c), Some(0));
    }

    #[test]
    fn backward() {
        let s = sheet();
        let re = Regex::new("^ap").unwrap();
        let c = CancelToken::new();
        assert_eq!(search(&s, &re, SearchScope::AllCols, 0, true, &c), Some(3));
    }

    #[test]
    fn cancelled_search_returns_none() {
        let s = sheet();
        let re = Regex::new("^ap").unwrap();
        let c = CancelToken::new();
        c.cancel();
        assert_eq!(search(&s, &re, SearchScope::AllCols, 0, false, &c), None);
    }

    #[test]
    fn select_matches() {
        let s = sheet();
        let re = Regex::new("an").unwrap();
        assert_eq!(matching_rowids(&s, &re, SearchScope::AllCols), vec![1]);
    }
}

//! Fixed-width loader: column boundaries detected from runs of spaces shared
//! across sample lines (visidata's fixed_width approach).

use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;
use rustidata_core::column::Column;
use rustidata_core::decompress;
use rustidata_core::sheet::Sheet;
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::Value;

use crate::Notify;

/// Find column spans: a column break is a position where every sample line
/// has a space (or has ended).
fn detect_spans(samples: &[String]) -> Vec<(usize, usize)> {
    let maxw = samples.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    if maxw == 0 {
        return vec![];
    }
    let grids: Vec<Vec<char>> = samples.iter().map(|l| l.chars().collect()).collect();
    let mut is_break = vec![true; maxw];
    for g in &grids {
        for (i, c) in g.iter().enumerate() {
            if *c != ' ' {
                is_break[i] = false;
            }
        }
    }
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..maxw {
        if !is_break[i] {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            spans.push((s, i));
        }
    }
    if let Some(s) = start {
        spans.push((s, maxw));
    }
    // merge spans separated by a single break column (within-name spaces)
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 - last.1 <= 1 => last.1 = span.1,
            _ => merged.push(span),
        }
    }
    merged
}

fn slice_chars(line: &str, start: usize, end: usize) -> String {
    line.chars().skip(start).take(end - start).collect::<String>().trim().to_string()
}

pub fn load_fixed(path: &Path, notify: Notify) -> Result<Sheet> {
    let reader = BufReader::new(decompress::open_reader(path)?);
    let lines: Vec<String> = reader.lines().collect::<std::io::Result<_>>()?;
    let samples: Vec<String> = lines.iter().take(100).cloned().collect();
    let spans = detect_spans(&samples);

    let headers: Vec<String> = match lines.first() {
        Some(first) => spans
            .iter()
            .enumerate()
            .map(|(i, &(s, e))| {
                let h = slice_chars(first, s, e);
                if h.is_empty() {
                    format!("col{}", i)
                } else {
                    h
                }
            })
            .collect(),
        None => vec![],
    };

    let rows: Vec<Vec<Value>> = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            spans
                .iter()
                .map(|&(s, e)| {
                    let cell = slice_chars(l, s, e);
                    if cell.is_empty() {
                        Value::Null
                    } else {
                        Value::Str(cell.into())
                    }
                })
                .collect()
        })
        .collect();

    let ncols = spans.len().max(1);
    let store = ColumnarStore::from_rows(ncols, rows);
    let columns = headers
        .into_iter()
        .enumerate()
        .map(|(i, h)| Column::source(h, i))
        .collect();
    notify();
    Ok(Sheet::new(
        crate::sheet_name(path),
        std::sync::Arc::new(store),
        columns,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_columns() {
        let mut f = tempfile::NamedTempFile::with_suffix(".fixed").unwrap();
        write!(
            f,
            "name    age  city\nalice   30   nyc\nbob     25   sf\n"
        )
        .unwrap();
        f.flush().unwrap();
        let s = load_fixed(f.path(), std::sync::Arc::new(|| {})).unwrap();
        assert_eq!(s.columns.len(), 3);
        assert_eq!(s.columns[0].name, "name");
        assert_eq!(s.nrows(), 2);
        assert_eq!(s.typed_at(0, 0), Value::Str("alice".into()));
        assert_eq!(s.typed_at(1, 2), Value::Str("sf".into()));
    }
}

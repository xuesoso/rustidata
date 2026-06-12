//! Excel loader via calamine. The first worksheet loads directly; multi-sheet
//! workbooks could later surface an index sheet.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use calamine::{Data, Reader};
use rustidata_core::column::Column;
use rustidata_core::sheet::Sheet;
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::Value;

fn cell_to_value(d: &Data) -> Value {
    match d {
        Data::Empty => Value::Null,
        Data::String(s) => Value::Str(s.as_str().into()),
        Data::Int(i) => Value::Int(*i),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                Value::Int(*f as i64)
            } else {
                Value::Float(*f)
            }
        }
        Data::Bool(b) => Value::Bool(*b),
        Data::DateTime(dt) => {
            // excel serial date -> epoch micros
            match dt.as_datetime() {
                Some(naive) => Value::Date(naive.and_utc().timestamp_micros()),
                None => Value::Float(dt.as_f64()),
            }
        }
        Data::DateTimeIso(s) => match rustidata_core::value::parse_date(s) {
            Some(us) => Value::Date(us),
            None => Value::Str(s.as_str().into()),
        },
        Data::DurationIso(s) => Value::Str(s.as_str().into()),
        Data::Error(e) => Value::Error(Arc::from(format!("{:?}", e).as_str())),
    }
}

pub fn load(path: &Path) -> Result<Sheet> {
    let mut workbook = calamine::open_workbook_auto(path)?;
    let sheet_names = workbook.sheet_names().to_vec();
    let first = sheet_names.first().context("workbook has no sheets")?;
    let range = workbook.worksheet_range(first)?;

    let mut rows_iter = range.rows();
    let headers: Vec<String> = match rows_iter.next() {
        Some(hdr) => hdr
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let s = match c {
                    Data::Empty => String::new(),
                    other => other.to_string(),
                };
                if s.is_empty() {
                    format!("col{}", i)
                } else {
                    s
                }
            })
            .collect(),
        None => vec![],
    };

    let ncols = headers.len().max(1);
    let rows: Vec<Vec<Value>> = rows_iter
        .map(|r| r.iter().map(cell_to_value).collect())
        .collect();

    let store = ColumnarStore::from_rows(ncols, rows);
    let columns: Vec<Column> = headers
        .into_iter()
        .enumerate()
        .map(|(i, h)| Column::source(h, i))
        .collect();
    Ok(Sheet::new(crate::sheet_name(path), Arc::new(store), columns))
}

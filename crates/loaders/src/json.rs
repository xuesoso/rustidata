//! JSON and JSONL loaders. Columns are discovered from object keys; types
//! inferred from JSON values (visidata's InferColumnsSheet behavior).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rustidata_core::column::Column;
use rustidata_core::decompress;
use rustidata_core::sheet::Sheet;
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::Value;
use serde_json::Value as Json;

use crate::Notify;

pub fn json_to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Json::String(s) => Value::Str(s.as_str().into()),
        // nested structures render as compact JSON text (unfurl expands them)
        other => Value::Str(other.to_string().into()),
    }
}

/// Discover columns across objects; rows are realized against the final
/// column list, padding missing keys with Null.
struct ColumnDiscovery {
    names: Vec<String>,
    index: HashMap<String, usize>,
}

impl ColumnDiscovery {
    fn new() -> Self {
        ColumnDiscovery {
            names: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn row_from(&mut self, j: &Json) -> Vec<(usize, Value)> {
        match j {
            Json::Object(map) => map
                .iter()
                .filter_map(|(k, v)| {
                    let idx = match self.index.get(k) {
                        Some(&i) => i,
                        None => {
                            // cap discovered columns (DoS hardening)
                            if self.names.len() >= rustidata_core::MAX_COLUMNS {
                                return None;
                            }
                            self.index.insert(k.clone(), self.names.len());
                            self.names.push(k.clone());
                            self.names.len() - 1
                        }
                    };
                    Some((idx, json_to_value(v)))
                })
                .collect(),
            other => {
                let idx = match self.index.get("value") {
                    Some(&i) => i,
                    None => {
                        self.index.insert("value".into(), self.names.len());
                        self.names.push("value".into());
                        self.names.len() - 1
                    }
                };
                vec![(idx, json_to_value(other))]
            }
        }
    }
}

/// Whole-document JSON: an array of objects (or a single object).
pub fn load_json(path: &Path) -> Result<Sheet> {
    let reader = decompress::open_reader(path)?;
    let doc: Json = serde_json::from_reader(reader)?;
    let items: Vec<Json> = match doc {
        Json::Array(items) => items,
        other => vec![other],
    };

    let mut disc = ColumnDiscovery::new();
    let sparse: Vec<Vec<(usize, Value)>> = items.iter().map(|j| disc.row_from(j)).collect();
    let ncols = disc.names.len().max(1);
    let rows: Vec<Vec<Value>> = sparse
        .into_iter()
        .map(|cells| {
            let mut row = vec![Value::Null; ncols];
            for (i, v) in cells {
                row[i] = v;
            }
            row
        })
        .collect();

    let store = ColumnarStore::from_rows(ncols, rows);
    let columns = if disc.names.is_empty() {
        vec![Column::source("value", 0)]
    } else {
        disc.names
            .iter()
            .enumerate()
            .map(|(i, n)| Column::source(n.clone(), i))
            .collect()
    };
    Ok(Sheet::new(crate::sheet_name(path), Arc::new(store), columns))
}

/// JSONL/NDJSON: stream line-by-line in the background.
///
/// Column discovery quirk: columns are fixed from the first 1000 lines (or
/// the whole file if shorter); later novel keys are dropped. This keeps the
/// store appendable while loading.
pub fn load_jsonl(path: &Path, notify: Notify) -> Result<Sheet> {
    let reader = BufReader::with_capacity(1 << 20, decompress::open_reader(path)?);
    let mut lines = reader.lines();

    // discovery phase
    let mut disc = ColumnDiscovery::new();
    let mut pending: Vec<Vec<(usize, Value)>> = Vec::new();
    for line in lines.by_ref().take(1000) {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(j) = serde_json::from_str::<Json>(&line) {
            pending.push(disc.row_from(&j));
        }
    }

    let ncols = disc.names.len().max(1);
    let store = Arc::new(ColumnarStore::new_any(ncols));
    let columns: Vec<Column> = if disc.names.is_empty() {
        vec![Column::source("value", 0)]
    } else {
        disc.names
            .iter()
            .enumerate()
            .map(|(i, n)| Column::source(n.clone(), i))
            .collect()
    };

    let realize = move |cells: Vec<(usize, Value)>, ncols: usize| -> Vec<Value> {
        let mut row = vec![Value::Null; ncols];
        for (i, v) in cells {
            if i < ncols {
                row[i] = v;
            }
        }
        row
    };

    let st = store.clone();
    let index = disc.index.clone();
    std::thread::Builder::new()
        .name("rd-jsonl".into())
        .spawn(move || {
            st.append_batch(pending.into_iter().map(|cells| realize(cells, ncols)));
            notify();
            let mut batch: Vec<Vec<Value>> = Vec::with_capacity(8192);
            for line in lines {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(&line) {
                    let mut row = vec![Value::Null; ncols];
                    for (k, v) in map {
                        if let Some(&i) = index.get(&k) {
                            row[i] = json_to_value(&v);
                        }
                    }
                    batch.push(row);
                    if batch.len() >= 8192 {
                        st.append_batch(batch.drain(..));
                        notify();
                    }
                }
            }
            if !batch.is_empty() {
                st.append_batch(batch.drain(..));
            }
            st.finish_loading();
            notify();
        })?;

    Ok(Sheet::new(crate::sheet_name(path), store, columns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn json_array_of_objects() {
        let mut f = tempfile::NamedTempFile::with_suffix(".json").unwrap();
        write!(
            f,
            r#"[{{"a": 1, "b": "x"}}, {{"a": 2.5, "c": true}}]"#
        )
        .unwrap();
        f.flush().unwrap();
        let s = load_json(f.path()).unwrap();
        assert_eq!(s.nrows(), 2);
        assert_eq!(s.columns.len(), 3);
        assert_eq!(s.typed_at(0, 0), Value::Int(1));
        assert_eq!(s.typed_at(1, 0), Value::Float(2.5));
        assert_eq!(s.typed_at(0, 2), Value::Null); // c missing in row 0
        assert_eq!(s.typed_at(1, 2), Value::Bool(true));
    }

    #[test]
    fn jsonl_streaming() {
        let mut f = tempfile::NamedTempFile::with_suffix(".jsonl").unwrap();
        writeln!(f, r#"{{"x": 1}}"#).unwrap();
        writeln!(f, r#"{{"x": 2, "y": "b"}}"#).unwrap();
        f.flush().unwrap();
        let s = load_jsonl(f.path(), Arc::new(|| {})).unwrap();
        for _ in 0..200 {
            if s.loading().is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(s.nrows(), 2);
        assert_eq!(s.typed_at(1, 1), Value::Str("b".into()));
    }

    #[test]
    fn nested_values_stringified() {
        let mut f = tempfile::NamedTempFile::with_suffix(".json").unwrap();
        write!(f, r#"[{{"a": [1, 2]}}]"#).unwrap();
        f.flush().unwrap();
        let s = load_json(f.path()).unwrap();
        assert_eq!(s.typed_at(0, 0), Value::Str("[1,2]".into()));
    }
}

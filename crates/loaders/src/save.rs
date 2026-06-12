//! Savers: csv, tsv, json, jsonl, markdown. Values are written through each
//! column's type/format (visidata's iterdispvals behavior), applying the
//! overlay and current sort order.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{bail, Result};
use rustidata_core::sheet::Sheet;
use rustidata_core::value::Value;

pub fn save(sheet: &Sheet, path: &Path) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("csv") => save_delim(sheet, path, b','),
        Some("tsv") => save_delim(sheet, path, b'\t'),
        Some("json") => save_json(sheet, path),
        Some("jsonl") | Some("ndjson") => save_jsonl(sheet, path),
        Some("md") | Some("markdown") => save_markdown(sheet, path),
        other => bail!("unsupported save format: {:?}", other.unwrap_or("none")),
    }
}

fn visible_headers(sheet: &Sheet) -> Vec<String> {
    sheet
        .visible_cols()
        .iter()
        .map(|&c| sheet.columns[c].name.clone())
        .collect()
}

fn save_delim(sheet: &Sheet, path: &Path, delim: u8) -> Result<()> {
    let mut w = csv::WriterBuilder::new()
        .delimiter(delim)
        .from_path(path)?;
    w.write_record(visible_headers(sheet))?;
    let vis = sheet.visible_cols();
    for rid in sheet.iter_rowids() {
        let rec: Vec<String> = vis
            .iter()
            .map(|&c| sheet.columns[c].display_value(sheet, rid).to_string())
            .collect();
        w.write_record(rec)?;
    }
    w.flush()?;
    Ok(())
}

fn typed_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::json!(i),
        Value::Float(f) => serde_json::json!(f),
        Value::Date(_) => serde_json::Value::String(v.to_display().to_string()),
        Value::Str(s) => serde_json::Value::String(s.to_string()),
        Value::Error(e) => serde_json::Value::String(e.to_string()),
    }
}

fn row_object(sheet: &Sheet, rid: rustidata_core::store::RowId, vis: &[usize]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for &c in vis {
        let col = &sheet.columns[c];
        obj.insert(
            col.name.clone(),
            typed_to_json(&col.typed_value(sheet, rid)),
        );
    }
    serde_json::Value::Object(obj)
}

fn save_json(sheet: &Sheet, path: &Path) -> Result<()> {
    let mut w = BufWriter::new(std::fs::File::create(path)?);
    let vis = sheet.visible_cols();
    let items: Vec<serde_json::Value> = sheet
        .iter_rowids()
        .map(|rid| row_object(sheet, rid, &vis))
        .collect();
    serde_json::to_writer_pretty(&mut w, &items)?;
    w.flush()?;
    Ok(())
}

fn save_jsonl(sheet: &Sheet, path: &Path) -> Result<()> {
    let mut w = BufWriter::new(std::fs::File::create(path)?);
    let vis = sheet.visible_cols();
    for rid in sheet.iter_rowids() {
        serde_json::to_writer(&mut w, &row_object(sheet, rid, &vis))?;
        w.write_all(b"\n")?;
    }
    w.flush()?;
    Ok(())
}

fn save_markdown(sheet: &Sheet, path: &Path) -> Result<()> {
    let mut w = BufWriter::new(std::fs::File::create(path)?);
    let headers = visible_headers(sheet);
    writeln!(w, "| {} |", headers.join(" | "))?;
    writeln!(
        w,
        "|{}|",
        headers.iter().map(|_| " --- ").collect::<Vec<_>>().join("|")
    )?;
    let vis = sheet.visible_cols();
    for rid in sheet.iter_rowids() {
        let cells: Vec<String> = vis
            .iter()
            .map(|&c| {
                sheet.columns[c]
                    .display_value(sheet, rid)
                    .replace('|', "\\|")
            })
            .collect();
        writeln!(w, "| {} |", cells.join(" | "))?;
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustidata_core::column::Column;
    use rustidata_core::store::ColumnarStore;
    use std::sync::Arc;

    fn sheet() -> Sheet {
        let store = ColumnarStore::from_rows(
            2,
            vec![
                vec![Value::Str("a".into()), Value::Int(1)],
                vec![Value::Str("b".into()), Value::Int(2)],
            ],
        );
        Sheet::new(
            "t",
            Arc::new(store),
            vec![Column::source("k", 0), Column::source("v", 1)],
        )
    }

    #[test]
    fn csv_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.csv");
        save(&sheet(), &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert_eq!(content, "k,v\na,1\nb,2\n");
    }

    #[test]
    fn jsonl_save() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.jsonl");
        save(&sheet(), &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        let first: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(first["k"], "a");
    }

    #[test]
    fn markdown_save() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.md");
        save(&sheet(), &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.starts_with("| k | v |"));
    }

    #[test]
    fn respects_overlay_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = sheet();
        s.overlay.set_cell(0, 1, Value::Int(99));
        let p = dir.path().join("out.csv");
        save(&s, &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("a,99"));
    }

    #[test]
    fn unsupported_format_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save(&sheet(), &dir.path().join("out.xyz")).is_err());
    }
}

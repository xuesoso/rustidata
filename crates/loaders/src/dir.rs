//! Directory sheet (visidata's DirSheet): browse files; Enter opens.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rustidata_core::column::Column;
use rustidata_core::sheet::{Sheet, SheetKind};
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::{VType, Value};

pub fn load_dir(path: &Path) -> Result<Sheet> {
    let mut entries: Vec<(String, String, i64, i64)> = Vec::new(); // name, type, size, mtime_us
    for entry in std::fs::read_dir(path)? {
        let Ok(entry) = entry else { continue };
        let meta = entry.metadata().ok();
        let ftype = match &meta {
            Some(m) if m.is_dir() => "dir",
            Some(m) if m.is_symlink() => "link",
            _ => "file",
        };
        let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        entries.push((
            entry.file_name().to_string_lossy().to_string(),
            ftype.to_string(),
            size,
            mtime,
        ));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let rows: Vec<Vec<Value>> = entries
        .into_iter()
        .map(|(name, ftype, size, mtime)| {
            vec![
                Value::Str(name.into()),
                Value::Str(ftype.into()),
                Value::Int(size),
                Value::Date(mtime),
            ]
        })
        .collect();

    let store = ColumnarStore::from_rows(4, rows);
    let columns = vec![
        Column::source("name", 0),
        Column::source("type", 1),
        Column::source("size", 2).with_type(VType::Int),
        Column::source("modified", 3).with_type(VType::Date),
    ];
    let mut sheet = Sheet::new(
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        Arc::new(store),
        columns,
    );
    sheet.kind = SheetKind::Dir;
    sheet.source_path = Some(path.to_path_buf());
    Ok(sheet)
}

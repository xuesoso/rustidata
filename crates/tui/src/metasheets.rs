//! Metasheets: sheets-sheet (S), columns-sheet (C), options-sheet (O), help.

use std::sync::Arc;

use rustidata_core::column::Column;
use rustidata_core::sheet::{Sheet, SheetKind};
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::{VType, Value};

use crate::app::App;

pub fn sheets_sheet(app: &App) -> Sheet {
    let rows: Vec<Vec<Value>> = app
        .sheets
        .iter()
        .rev()
        .map(|s| {
            vec![
                Value::Str(s.name.as_str().into()),
                Value::Str(format!("{:?}", s.kind).into()),
                Value::Int(s.nrows() as i64),
                Value::Int(s.columns.len() as i64),
                Value::Str(
                    s.source_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        .into(),
                ),
                Value::Int(s.id as i64),
            ]
        })
        .collect();
    let store = ColumnarStore::from_rows(6, rows);
    let columns = vec![
        Column::source("name", 0),
        Column::source("kind", 1),
        Column::source("rows", 2).with_type(VType::Int),
        Column::source("cols", 3).with_type(VType::Int),
        Column::source("source", 4),
        Column::source("id", 5).with_type(VType::Int),
    ];
    let mut sheet = Sheet::new("sheets", Arc::new(store), columns);
    sheet.kind = SheetKind::Sheets;
    sheet
}

pub fn columns_sheet(parent: &Sheet) -> Sheet {
    let rows: Vec<Vec<Value>> = parent
        .columns
        .iter()
        .map(|c| {
            vec![
                Value::Str(c.name.as_str().into()),
                Value::Str(c.vtype.name().into()),
                match c.width {
                    Some(w) => Value::Int(w as i64),
                    None => Value::Null,
                },
                Value::Bool(c.is_key()),
                Value::Bool(c.hidden),
            ]
        })
        .collect();
    let store = ColumnarStore::from_rows(5, rows);
    let columns = vec![
        Column::source("column", 0),
        Column::source("type", 1),
        Column::source("width", 2).with_type(VType::Int),
        Column::source("key", 3),
        Column::source("hidden", 4),
    ];
    let mut sheet = Sheet::new(format!("{}_columns", parent.name), Arc::new(store), columns);
    sheet.kind = SheetKind::Columns;
    sheet.parent = Some(parent.id);
    sheet
}

pub fn options_sheet(app: &App) -> Sheet {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    // flatten core options through TOML so names match the config file
    if let Ok(toml_str) = toml::to_string(&app.opts) {
        if let Ok(table) = toml_str.parse::<toml::Table>() {
            for (k, v) in table {
                rows.push(vec![
                    Value::Str(k.into()),
                    Value::Str(v.to_string().trim_matches('"').into()),
                ]);
            }
        }
    }
    rows.push(vec![
        Value::Str("theme".into()),
        Value::Str(app.theme.name.into()),
    ]);
    let store = ColumnarStore::from_rows(2, rows);
    let columns = vec![Column::source("option", 0), Column::source("value", 1)];
    let mut sheet = Sheet::new("options", Arc::new(store), columns);
    sheet.kind = SheetKind::Options;
    sheet
}

pub fn help_sheet() -> Sheet {
    let rows: Vec<Vec<Value>> = crate::commands::BINDINGS
        .iter()
        .map(|(ks, ln, help)| {
            vec![
                Value::Str((*ks).into()),
                Value::Str((*ln).into()),
                Value::Str((*help).into()),
            ]
        })
        .collect();
    let store = ColumnarStore::from_rows(3, rows);
    let columns = vec![
        Column::source("key", 0).with_width(8),
        Column::source("command", 1).with_width(18),
        Column::source("description", 2),
    ];
    let mut sheet = Sheet::new("help", Arc::new(store), columns);
    sheet.kind = SheetKind::Help;
    sheet
}

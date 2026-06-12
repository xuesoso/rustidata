//! Plain-text sheet: one row per line, single "text" column (visidata's
//! TextSheet). Uses the mmap store with a fake no-delimiter dialect so huge
//! logs open instantly.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rustidata_core::column::Column;
use rustidata_core::sheet::{Sheet, SheetKind};
use rustidata_core::store::{ColumnarStore, RowSource};
use rustidata_core::value::Value;

pub fn load_text(path: &Path) -> Result<Sheet> {
    // \x00 as delimiter => whole line is one field; no header row
    let store = rustidata_core::store::MmapTextStore::open_with(path, Some(b'\x00'), Some(false))?;
    let store = Arc::new(store);
    if store.loading().is_some() {
        let st = store.clone();
        std::thread::spawn(move || {
            let cancel = std::sync::atomic::AtomicBool::new(false);
            st.index_remainder(&cancel, || {});
        });
    }
    let columns = vec![Column::source("text", 0)];
    let mut sheet = Sheet::new(crate::sheet_name(path), store, columns);
    sheet.kind = SheetKind::Text;
    Ok(sheet)
}

/// Build a text sheet from in-memory lines (errors, help blobs).
pub fn text_sheet_from_lines(name: &str, lines: Vec<String>) -> Sheet {
    let rows: Vec<Vec<Value>> = lines
        .into_iter()
        .map(|l| vec![Value::Str(l.into())])
        .collect();
    let store = ColumnarStore::from_rows(1, rows);
    let mut sheet = Sheet::new(name, Arc::new(store), vec![Column::source("text", 0)]);
    sheet.kind = SheetKind::Text;
    sheet
}

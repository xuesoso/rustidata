//! Internal clipboard for rows/cells, with optional system clipboard sync.

use rustidata_core::value::Value;

#[derive(Default)]
pub struct Clipboard {
    /// Copied rows as realized values (column-name, value) pairs are not
    /// needed; paste targets positional columns.
    pub rows: Vec<Vec<Value>>,
    pub cell: Option<Value>,
}

impl Clipboard {
    pub fn copy_cell(&mut self, v: Value) {
        self.cell = Some(v.clone());
        sys_set(&v.to_display());
    }

    pub fn copy_rows(&mut self, rows: Vec<Vec<Value>>) {
        if !rows.is_empty() {
            let text = rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|v| v.to_display().to_string())
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n");
            sys_set(&text);
        }
        self.rows = rows;
    }
}

#[cfg(feature = "system-clipboard")]
fn sys_set(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_string());
    }
}

#[cfg(not(feature = "system-clipboard"))]
fn sys_set(_text: &str) {}

/// Paste source: system clipboard text, if available.
#[cfg(feature = "system-clipboard")]
#[allow(dead_code)] // reserved for paste-from-system
pub fn sys_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

#[cfg(not(feature = "system-clipboard"))]
#[allow(dead_code)]
pub fn sys_get() -> Option<String> {
    None
}

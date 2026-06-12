//! Columns: a named, typed view over a store field, an expression, or a
//! computed function.

use std::sync::Arc;

use compact_str::CompactString;
use parking_lot::Mutex;

use crate::sheet::Sheet;
use crate::store::{RawCell, RowId};
use crate::value::{VType, Value};

pub type ComputedFn = Arc<dyn Fn(&Sheet, RowId) -> Value + Send + Sync>;

#[derive(Clone)]
pub enum ColumnKind {
    /// Index into the RowSource's fields.
    Source(usize),
    /// Rhai expression over the row's other columns.
    Expr {
        src: String,
        ast: Arc<rhai::AST>,
        cache: Arc<Mutex<lru::LruCache<RowId, Value>>>,
    },
    /// Materialized values keyed by RowId (freeze results, paste targets).
    Frozen(Arc<Mutex<std::collections::HashMap<RowId, Value>>>),
    /// Arbitrary computed value (metasheets, describe).
    Computed(ComputedFn),
}

#[derive(Clone)]
pub struct Column {
    pub name: String,
    pub kind: ColumnKind,
    pub vtype: VType,
    /// None = auto-size from visible rows; Some(0) is not used (hide is a flag).
    pub width: Option<u16>,
    pub fmtstr: Option<String>,
    pub hidden: bool,
    /// Key columns are pinned left in order; visidata's `!`.
    pub key_pos: Option<u8>,
}

impl Column {
    pub fn source(name: impl Into<String>, idx: usize) -> Self {
        Column {
            name: name.into(),
            kind: ColumnKind::Source(idx),
            vtype: VType::Any,
            width: None,
            fmtstr: None,
            hidden: false,
            key_pos: None,
        }
    }

    pub fn expr(name: impl Into<String>, src: impl Into<String>, ast: Arc<rhai::AST>) -> Self {
        Column {
            name: name.into(),
            kind: ColumnKind::Expr {
                src: src.into(),
                ast,
                cache: Arc::new(Mutex::new(lru::LruCache::new(
                    std::num::NonZeroUsize::new(8192).unwrap(),
                ))),
            },
            vtype: VType::Any,
            width: None,
            fmtstr: None,
            hidden: false,
            key_pos: None,
        }
    }

    pub fn computed(name: impl Into<String>, f: ComputedFn) -> Self {
        Column {
            name: name.into(),
            kind: ColumnKind::Computed(f),
            vtype: VType::Any,
            width: None,
            fmtstr: None,
            hidden: false,
            key_pos: None,
        }
    }

    pub fn with_type(mut self, vtype: VType) -> Self {
        self.vtype = vtype;
        self
    }

    pub fn with_width(mut self, w: u16) -> Self {
        self.width = Some(w);
        self
    }

    pub fn is_key(&self) -> bool {
        self.key_pos.is_some()
    }

    /// The raw (untyped) value of this column for a row.
    pub fn raw_value(&self, sheet: &Sheet, row: RowId) -> Value {
        match &self.kind {
            ColumnKind::Source(idx) => sheet.raw_cell(row, *idx).into_value(),
            ColumnKind::Frozen(map) => map.lock().get(&row).cloned().unwrap_or(Value::Null),
            ColumnKind::Computed(f) => f(sheet, row),
            ColumnKind::Expr { ast, cache, .. } => {
                if let Some(v) = cache.lock().get(&row) {
                    return v.clone();
                }
                let v = crate::expr::eval_row(sheet, row, ast);
                cache.lock().put(row, v.clone());
                v
            }
        }
    }

    /// Typed value: raw value passed through this column's VType.
    pub fn typed_value(&self, sheet: &Sheet, row: RowId) -> Value {
        let raw = match &self.kind {
            ColumnKind::Source(idx) => {
                // fast path: parse text directly without intermediate Value
                match sheet.raw_cell(row, *idx) {
                    RawCell::Text(s) => return self.vtype.parse(&s),
                    other => other.into_value(),
                }
            }
            _ => self.raw_value(sheet, row),
        };
        self.vtype.coerce(raw)
    }

    /// Final display string for a cell.
    pub fn display_value(&self, sheet: &Sheet, row: RowId) -> CompactString {
        let v = self.typed_value(sheet, row);
        self.vtype.format(&v, self.fmtstr.as_deref())
    }

    /// Header text including the type note suffix, visidata-style.
    pub fn header_text(&self) -> String {
        let note = self.vtype.note();
        if note.is_empty() {
            self.name.clone()
        } else {
            format!("{}{}", self.name, note)
        }
    }

    pub fn invalidate_cache(&self) {
        if let ColumnKind::Expr { cache, .. } = &self.kind {
            cache.lock().clear();
        }
    }
}

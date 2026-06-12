//! rustidata-core: the data engine.
//!
//! Headless (no terminal dependency) so transforms, stores, and the expression
//! engine are testable and benchmarkable in isolation.

pub mod aggregate;
pub mod column;
pub mod decompress;
pub mod expr;
pub mod jobs;
pub mod options;
pub mod search;
pub mod sheet;
pub mod sniff;
pub mod sort;
pub mod store;
pub mod transform;
pub mod undo;
pub mod value;

// re-exported for downstream crates (loaders write compressed test fixtures)
pub use flate2;

/// Hard cap on columns created from file content (DoS hardening: a malicious
/// header line with millions of fields must not allocate unbounded memory).
pub const MAX_COLUMNS: usize = 10_000;

pub use column::{Column, ColumnKind};
pub use sheet::{Sheet, SheetId, SheetKind};
pub use store::{ColumnarStore, MmapTextStore, Overlay, Progress, RawCell, RowId, RowSource};
pub use value::{VType, Value};

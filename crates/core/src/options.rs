//! Options, with visidata-style names where they overlap.
//!
//! Resolution order is default -> config file -> CLI; per-sheet overrides can
//! be layered by cloning and mutating a sheet's copy.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Forced delimiter; empty = sniff automatically.
    pub delimiter: String,
    /// Default column display width.
    pub default_width: u16,
    /// Hard cap for auto-sized columns.
    pub max_width: u16,
    /// Glyph for truncated cells.
    pub disp_truncator: String,
    /// What to show for null cells.
    pub disp_null: String,
    /// Column separator glyph.
    pub disp_column_sep: String,
    /// Total gap between columns in cells; the separator glyph sits centered
    /// in it. 0 = no gap or separator.
    pub col_gap: u16,
    /// Key-column separator glyph.
    pub disp_keycol_sep: String,
    /// Default date format string.
    pub fmt_date: String,
    /// Treat first row as header (auto = sniff).
    pub header: HeaderMode,
    /// Rows of slack kept above/below cursor when scrolling.
    pub scroll_context: u64,
    /// Scatter-plot point radius in braille subpixels (0 = bare dot). Applied
    /// symmetrically on both axes, so the dot is a filled disc of this radius.
    pub plot_dot_radius: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeaderMode {
    Auto,
    Always,
    Never,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            delimiter: String::new(),
            default_width: 20,
            max_width: 50,
            disp_truncator: "…".into(),
            disp_null: "".into(),
            disp_column_sep: "│".into(),
            col_gap: 3,
            disp_keycol_sep: "║".into(),
            fmt_date: "%Y-%m-%d %H:%M:%S".into(),
            header: HeaderMode::Auto,
            scroll_context: 0,
            plot_dot_radius: 1,
        }
    }
}

impl Options {
    pub fn forced_delimiter(&self) -> Option<u8> {
        match self.delimiter.as_str() {
            "" => None,
            "\\t" | "tab" => Some(b'\t'),
            s => s.as_bytes().first().copied(),
        }
    }
}

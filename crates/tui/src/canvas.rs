//! Braille scatter plots (visidata's canvas/graph): 2x4 subpixels per cell
//! via U+2800..U+28FF, with axes, legend, zoom and pan.

use std::collections::HashMap;

use crossterm::style::Color;
use rustidata_core::sheet::{Sheet, SheetId, SheetKind};
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::Value;

use crate::app::App;
use crate::screen::ScreenBuffer;
use crate::theme::{Style, StyleKey, Theme};

/// Braille dot bit for subpixel (dx in 0..2, dy in 0..4).
fn dot_bit(dx: u32, dy: u32) -> u8 {
    match (dx, dy) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        _ => 0,
    }
}

pub struct BrailleGrid {
    w: usize, // cells
    h: usize,
    bits: Vec<u8>,
    series: Vec<u8>, // last series to draw in each cell (for color)
}

impl BrailleGrid {
    pub fn new(w: usize, h: usize) -> Self {
        BrailleGrid {
            w,
            h,
            bits: vec![0; w * h],
            series: vec![0; w * h],
        }
    }

    pub fn plot_pixel(&mut self, px: u32, py: u32, series: u8) {
        let cx = (px / 2) as usize;
        let cy = (py / 4) as usize;
        if cx >= self.w || cy >= self.h {
            return;
        }
        let i = cy * self.w + cx;
        self.bits[i] |= dot_bit(px % 2, py % 4);
        self.series[i] = series;
    }

    pub fn cell_char(&self, cx: usize, cy: usize) -> Option<(char, u8)> {
        let i = cy * self.w + cx;
        let b = self.bits[i];
        if b == 0 {
            return None;
        }
        Some((char::from_u32(0x2800 + b as u32)?, self.series[i]))
    }

    pub fn pixel_width(&self) -> u32 {
        (self.w * 2) as u32
    }

    pub fn pixel_height(&self) -> u32 {
        (self.h * 4) as u32
    }
}

pub struct GraphData {
    pub points: Vec<(f64, f64, u8)>,
    pub series_names: Vec<String>,
    pub xname: String,
    /// Visible bounds (zoom/pan state).
    pub xmin: f64,
    pub xmax: f64,
    pub ymin: f64,
    pub ymax: f64,
    /// Full data bounds (for reset).
    pub data_bounds: (f64, f64, f64, f64),
}

impl GraphData {
    fn from_points(
        points: Vec<(f64, f64, u8)>,
        series_names: Vec<String>,
        xname: String,
    ) -> Self {
        let xmin = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let xmax = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let ymin = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let ymax = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let (xmin, xmax) = pad_range(xmin, xmax);
        let (ymin, ymax) = pad_range(ymin, ymax);
        GraphData {
            points,
            series_names,
            xname,
            xmin,
            xmax,
            ymin,
            ymax,
            data_bounds: (xmin, xmax, ymin, ymax),
        }
    }

    pub fn zoom(&mut self, factor: f64) {
        let cx = (self.xmin + self.xmax) / 2.0;
        let cy = (self.ymin + self.ymax) / 2.0;
        let hw = (self.xmax - self.xmin) / 2.0 * factor;
        let hh = (self.ymax - self.ymin) / 2.0 * factor;
        self.xmin = cx - hw;
        self.xmax = cx + hw;
        self.ymin = cy - hh;
        self.ymax = cy + hh;
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        let w = self.xmax - self.xmin;
        let h = self.ymax - self.ymin;
        self.xmin += dx * w;
        self.xmax += dx * w;
        self.ymin += dy * h;
        self.ymax += dy * h;
    }

    pub fn reset(&mut self) {
        let (a, b, c, d) = self.data_bounds;
        self.xmin = a;
        self.xmax = b;
        self.ymin = c;
        self.ymax = d;
    }
}

fn pad_range(min: f64, max: f64) -> (f64, f64) {
    if !min.is_finite() || !max.is_finite() {
        return (0.0, 1.0);
    }
    if min == max {
        return (min - 0.5, max + 0.5);
    }
    let pad = (max - min) * 0.02;
    (min - pad, max + pad)
}

const SERIES_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Yellow,
    Color::Green,
    Color::Magenta,
    Color::Red,
    Color::Blue,
    Color::White,
];

/// Open a graph of the cursor column (y) against the first key column or the
/// row index (x). Multiple numeric y series: selected columns... v1 plots the
/// cursor column only; series split by first key column when categorical.
pub fn open_graph(app: &mut App) -> Result<(), anyhow::Error> {
    let Some(s) = app.sheet() else { return Ok(()) };
    let Some(yci) = s.cursor_col_index() else { return Ok(()) };
    if !s.columns[yci].vtype.is_numeric() {
        anyhow::bail!(
            "column '{}' is not numeric (set type with # or %)",
            s.columns[yci].name
        );
    }
    let key_cols: Vec<usize> = (0..s.columns.len())
        .filter(|&i| s.columns[i].is_key())
        .collect();
    // numeric key col -> x axis; else row index
    let xci = key_cols
        .iter()
        .copied()
        .find(|&i| s.columns[i].vtype.is_numeric() || s.columns[i].vtype == rustidata_core::value::VType::Date);

    let mut points = Vec::new();
    for (i, rid) in s.selected_or_all().into_iter().enumerate() {
        let y = match s.columns[yci].typed_value(s, rid).as_f64() {
            Some(v) => v,
            None => continue,
        };
        let x = match xci {
            Some(ci) => match s.columns[ci].typed_value(s, rid).as_f64() {
                Some(v) => v,
                None => continue,
            },
            None => i as f64,
        };
        points.push((x, y, 0u8));
    }
    if points.is_empty() {
        anyhow::bail!("no plottable points");
    }

    let yname = s.columns[yci].name.clone();
    let xname = xci
        .map(|ci| s.columns[ci].name.clone())
        .unwrap_or_else(|| "row".into());
    let title = format!("{}_vs_{}", yname, xname);
    let parent_id = s.id;

    let data = GraphData::from_points(points, vec![yname.clone()], xname);
    // placeholder backing sheet so the stack/q semantics work
    let store = ColumnarStore::from_rows(1, Vec::<Vec<Value>>::new());
    let mut sheet = Sheet::new(
        title,
        std::sync::Arc::new(store),
        vec![rustidata_core::column::Column::source("plot", 0)],
    );
    sheet.kind = SheetKind::Graph;
    sheet.parent = Some(parent_id);
    let gid = sheet.id;
    app.graphs.insert(gid, data);
    app.push_sheet(sheet);
    app.info("graph: hjkl pan, +/- zoom, 0 reset, q close");
    Ok(())
}

/// Graph-specific keys; returns true if handled.
pub fn handle_graph_key(app: &mut App, ks: &str) -> bool {
    let Some(s) = app.sheet() else { return false };
    if s.kind != SheetKind::Graph {
        return false;
    }
    let id = s.id;
    let Some(g) = app.graphs.get_mut(&id) else { return false };
    match ks {
        "+" => g.zoom(0.5),
        "-" => g.zoom(2.0),
        "0" => g.reset(),
        "h" | "Left" => g.pan(-0.1, 0.0),
        "l" | "Right" => g.pan(0.1, 0.0),
        "j" | "Down" => g.pan(0.0, -0.1),
        "k" | "Up" => g.pan(0.0, 0.1),
        "q" => {
            app.graphs.remove(&id);
            app.pop_sheet();
        }
        _ => return false,
    }
    app.dirty = true;
    true
}

pub fn draw_graph(
    sb: &mut ScreenBuffer,
    theme: &Theme,
    g: &GraphData,
    sheet_id: SheetId,
    y0: u16,
    height: u16,
) {
    let _ = sheet_id;
    let base = theme.get(StyleKey::Default);
    let axis = base.merge_over(&theme.get(StyleKey::GraphAxis));
    let width = sb.width();
    let left_margin: u16 = 10;
    let bottom_margin: u16 = 2;
    if width <= left_margin + 2 || height <= bottom_margin + 2 {
        return;
    }
    let plot_w = (width - left_margin - 1) as usize;
    let plot_h = (height - bottom_margin) as usize;

    let mut grid = BrailleGrid::new(plot_w, plot_h);
    let pw = grid.pixel_width() as f64;
    let ph = grid.pixel_height() as f64;
    for &(x, y, series) in &g.points {
        if x < g.xmin || x > g.xmax || y < g.ymin || y > g.ymax {
            continue; // clip to view
        }
        let px = ((x - g.xmin) / (g.xmax - g.xmin) * (pw - 1.0)) as u32;
        let py = ((g.ymax - y) / (g.ymax - g.ymin) * (ph - 1.0)) as u32;
        grid.plot_pixel(px, py, series);
    }

    // plot cells
    for cy in 0..plot_h {
        for cx in 0..plot_w {
            if let Some((ch, series)) = grid.cell_char(cx, cy) {
                let color = SERIES_COLORS[series as usize % SERIES_COLORS.len()];
                let style = Style {
                    fg: Some(color),
                    bg: base.bg,
                    attrs: base.attrs,
                };
                sb.put_str(left_margin + cx as u16, y0 + cy as u16, &ch.to_string(), style);
            }
        }
    }

    // y axis labels
    let ymax_label = format_axis(g.ymax);
    let ymin_label = format_axis(g.ymin);
    sb.put_str(0, y0, &format!("{:>9}┤", ymax_label), axis);
    sb.put_str(
        0,
        y0 + plot_h as u16 - 1,
        &format!("{:>9}┤", ymin_label),
        axis,
    );
    for cy in 1..plot_h.saturating_sub(1) {
        sb.put_str(left_margin - 1, y0 + cy as u16, "│", axis);
    }

    // x axis
    let xaxis_y = y0 + plot_h as u16;
    let bar: String = "─".repeat(plot_w);
    sb.put_str(left_margin, xaxis_y, &bar, axis);
    sb.put_str(left_margin - 1, xaxis_y, "└", axis);
    let xmin_label = format_axis(g.xmin);
    let xmax_label = format_axis(g.xmax);
    sb.put_str(left_margin, xaxis_y + 1, &xmin_label, axis);
    let xr = (left_margin as usize + plot_w).saturating_sub(xmax_label.len());
    sb.put_str(xr as u16, xaxis_y + 1, &xmax_label, axis);
    let xmid = left_margin as usize + plot_w / 2;
    sb.put_str(
        xmid.saturating_sub(g.xname.len() / 2) as u16,
        xaxis_y + 1,
        &g.xname,
        axis,
    );

    // legend (top right)
    let mut ly = y0;
    for (i, name) in g.series_names.iter().enumerate() {
        let color = SERIES_COLORS[i % SERIES_COLORS.len()];
        let style = Style {
            fg: Some(color),
            bg: base.bg,
            attrs: base.attrs,
        };
        let label = format!("⣿ {}", name);
        let lx = (width as usize).saturating_sub(label.len() + 2);
        sb.put_str(lx as u16, ly, &label, style);
        ly += 1;
    }
}

fn format_axis(v: f64) -> String {
    if v.abs() >= 1e6 || (v.abs() < 1e-3 && v != 0.0) {
        format!("{:.2e}", v)
    } else if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{:.2}", v)
    }
}

/// Per-app graph store (sheet id -> data); lives in App.
pub type GraphStore = HashMap<SheetId, GraphData>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braille_bits() {
        let mut g = BrailleGrid::new(2, 1);
        g.plot_pixel(0, 0, 0); // dot 1 -> 0x01
        g.plot_pixel(1, 3, 0); // dot 8 -> 0x80
        let (ch, _) = g.cell_char(0, 0).unwrap();
        assert_eq!(ch as u32, 0x2800 + 0x81);
        assert!(g.cell_char(1, 0).is_none());
    }

    #[test]
    fn bounds_pad() {
        let g = GraphData::from_points(
            vec![(0.0, 5.0, 0), (10.0, 15.0, 0)],
            vec!["y".into()],
            "x".into(),
        );
        assert!(g.xmin < 0.0 && g.xmax > 10.0);
        assert!(g.ymin < 5.0 && g.ymax > 15.0);
    }

    #[test]
    fn zoom_and_reset() {
        let mut g = GraphData::from_points(
            vec![(0.0, 0.0, 0), (10.0, 10.0, 0)],
            vec!["y".into()],
            "x".into(),
        );
        let w0 = g.xmax - g.xmin;
        g.zoom(0.5);
        assert!((g.xmax - g.xmin) < w0);
        g.reset();
        assert!(((g.xmax - g.xmin) - w0).abs() < 1e-9);
    }
}

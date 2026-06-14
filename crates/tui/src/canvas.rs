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

    /// Plot a point as a filled disc of subpixels centered on (px, py), using
    /// the same `radius` on both axes so the dot is symmetric. radius 0 lights
    /// a single subpixel (a bare dot).
    pub fn plot_point(&mut self, px: u32, py: u32, series: u8, radius: u32) {
        if radius == 0 {
            self.plot_pixel(px, py, series);
            return;
        }
        let r = radius as i64;
        let (cx, cy) = (px as i64, py as i64);
        for dy in -r..=r {
            for dx in -r..=r {
                // filled circle, equal extent each way
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let nx = cx + dx;
                let ny = cy + dy;
                if nx >= 0 && ny >= 0 {
                    self.plot_pixel(nx as u32, ny as u32, series);
                }
            }
        }
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
    /// True (unpadded) data value range, used to anchor axis tick labels so
    /// they never fall outside the actual distribution of the values.
    pub xdata: (f64, f64),
    pub ydata: (f64, f64),
    /// Subpixel radius of each plotted point (0 = bare dot).
    pub dot_radius: u32,
}

/// Default point radius: a small symmetric dot a few subpixels across.
const DEFAULT_DOT_RADIUS: u32 = 1;
const MAX_DOT_RADIUS: u32 = 16;

impl GraphData {
    fn from_points(
        points: Vec<(f64, f64, u8)>,
        series_names: Vec<String>,
        xname: String,
    ) -> Self {
        let xlo = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let xhi = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let ylo = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let yhi = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let (xmin, xmax) = pad_range(xlo, xhi);
        let (ymin, ymax) = pad_range(ylo, yhi);
        GraphData {
            points,
            series_names,
            xname,
            xmin,
            xmax,
            ymin,
            ymax,
            data_bounds: (xmin, xmax, ymin, ymax),
            xdata: (xlo, xhi),
            ydata: (ylo, yhi),
            dot_radius: DEFAULT_DOT_RADIUS,
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

    let mut data = GraphData::from_points(points, vec![yname.clone()], xname);
    data.dot_radius = app.opts.plot_dot_radius.min(MAX_DOT_RADIUS);
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
    app.info("graph: hjkl pan, +/- zoom, ,/. dot size, 0 reset, q close");
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
        "." => g.dot_radius = (g.dot_radius + 1).min(MAX_DOT_RADIUS),
        "," => g.dot_radius = g.dot_radius.saturating_sub(1),
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
    // axis line + tick-label row + axis-name row
    let bottom_margin: u16 = 3;
    if width <= left_margin + 2 || height <= bottom_margin + 2 {
        return;
    }
    let plot_w = (width - left_margin - 1) as usize;
    let plot_h = (height - bottom_margin) as usize;

    let mut grid = BrailleGrid::new(plot_w, plot_h);
    let pw = grid.pixel_width() as f64;
    let ph = grid.pixel_height() as f64;
    // Map a data value to a braille subpixel. Tick labels reuse these and then
    // divide by the cell's subpixel size (px/2, py/4 — see plot_pixel) so a
    // label lands on the exact cell its data value renders into.
    let px_of = |x: f64| ((x - g.xmin) / (g.xmax - g.xmin) * (pw - 1.0)) as u32;
    let py_of = |y: f64| ((g.ymax - y) / (g.ymax - g.ymin) * (ph - 1.0)) as u32;
    for &(x, y, series) in &g.points {
        if x < g.xmin || x > g.xmax || y < g.ymin || y > g.ymax {
            continue; // clip to view
        }
        grid.plot_point(px_of(x), py_of(y), series, g.dot_radius);
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

    // y axis: tick labels at the data-range quantiles that fall in view, with
    // the vertical bar (│) filling the rows between them.
    let mut yrow_label: Vec<Option<String>> = vec![None; plot_h];
    for val in axis_ticks(g.ydata.0, g.ydata.1, g.ymin, g.ymax) {
        let r = (py_of(val) / 4) as usize;
        if r < plot_h {
            yrow_label[r].get_or_insert_with(|| format_axis(val));
        }
    }
    for (cy, slot) in yrow_label.iter().enumerate() {
        let yy = y0 + cy as u16;
        match slot {
            Some(lbl) => {
                sb.put_str(0, yy, &format!("{:>9}┤", lbl), axis);
            }
            None => {
                sb.put_str(left_margin - 1, yy, "│", axis);
            }
        }
    }

    // x axis: tick marks (┬) on the baseline at the in-view data quantiles,
    // with labels centered below (greedy left-to-right to avoid overlap).
    let xaxis_y = y0 + plot_h as u16;
    let mut bar: Vec<char> = std::iter::repeat('─').take(plot_w).collect();
    let mut xticks: Vec<(usize, String)> = Vec::new();
    for val in axis_ticks(g.xdata.0, g.xdata.1, g.xmin, g.xmax) {
        let c = (px_of(val) / 2) as usize;
        if c < plot_w {
            bar[c] = '┬';
            if xticks.last().map_or(true, |&(pc, _)| pc != c) {
                xticks.push((c, format_axis(val)));
            }
        }
    }
    let bar_str: String = bar.into_iter().collect();
    sb.put_str(left_margin, xaxis_y, &bar_str, axis);
    sb.put_str(left_margin - 1, xaxis_y, "└", axis);
    let mut last_end: i32 = -1;
    for (c, lbl) in &xticks {
        let center = left_margin as usize + c;
        let start = center.saturating_sub(lbl.len() / 2);
        if start as i32 > last_end && start + lbl.len() <= width as usize {
            sb.put_str(start as u16, xaxis_y + 1, lbl, axis);
            last_end = (start + lbl.len()) as i32;
        }
    }
    let xmid = left_margin as usize + plot_w / 2;
    sb.put_str(
        xmid.saturating_sub(g.xname.len() / 2) as u16,
        xaxis_y + 2,
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

/// Tick values at 0/25/50/75/100% of the data range `[dmin, dmax]`, keeping
/// only those that fall inside the visible window `[vmin, vmax]`. Anchoring to
/// the data range (not the viewport) means ticks never land outside the actual
/// distribution: zooming out leaves them at the data quantiles rather than
/// scattering labels across empty space, and zooming in drops the ones that
/// scroll off screen.
fn axis_ticks(dmin: f64, dmax: f64, vmin: f64, vmax: f64) -> Vec<f64> {
    if !dmin.is_finite() || !dmax.is_finite() {
        return Vec::new();
    }
    let span = dmax - dmin;
    let eps = ((vmax - vmin).abs() * 1e-9).max(f64::EPSILON);
    (0..=4)
        .map(|k| dmin + span * (k as f64 / 4.0))
        .filter(|&val| val >= vmin - eps && val <= vmax + eps)
        .collect()
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
    fn plot_point_fattens() {
        // radius 0 lights one subpixel; radius 4 lights a neighborhood.
        let mut g0 = BrailleGrid::new(12, 10);
        g0.plot_point(20, 20, 0, 0);
        let lit0 = (0..g0.bits.len()).filter(|&i| g0.bits[i] != 0).count();
        assert_eq!(lit0, 1);

        let mut g = BrailleGrid::new(12, 10);
        g.plot_point(20, 20, 0, 4);
        assert!(g.cell_char(10, 5).is_some()); // center cell lit
        assert!(g.cell_char(0, 0).is_none()); // far corner untouched
        let lit = (0..g.bits.len()).filter(|&i| g.bits[i] != 0).count();
        assert!(lit > lit0, "radius 4 should light more cells than radius 0");
    }

    #[test]
    fn subpixel_maps_to_expected_cell() {
        // A subpixel lands in cell (px/2, py/4). Axis tick labels reuse this
        // same division to align with rendered point centers, so pin it here.
        let mut g = BrailleGrid::new(6, 6);
        g.plot_pixel(7, 13, 0); // cx = 7/2 = 3, cy = 13/4 = 3
        assert!(g.cell_char(3, 3).is_some());
        assert!(g.cell_char(0, 0).is_none());
    }

    #[test]
    fn dots_land_on_tick_cells() {
        // End-to-end: a point at a tick's value lights the exact cell that tick
        // labels. Mirrors draw_graph's value -> subpixel -> cell mapping, so it
        // breaks if the point and tick paths ever diverge.
        let (plot_w, plot_h) = (40usize, 30usize);
        let (xmin, xmax, ymin, ymax) = (-2.0f64, 102.0, -2.0, 102.0);
        let pw = (plot_w * 2) as f64;
        let ph = (plot_h * 4) as f64;
        let px_of = |x: f64| ((x - xmin) / (xmax - xmin) * (pw - 1.0)) as u32;
        let py_of = |y: f64| ((ymax - y) / (ymax - ymin) * (ph - 1.0)) as u32;

        let quantiles = [0.0, 25.0, 50.0, 75.0, 100.0];
        let mut grid = BrailleGrid::new(plot_w, plot_h);
        for &v in &quantiles {
            grid.plot_point(px_of(v), py_of(v), 0, DEFAULT_DOT_RADIUS);
        }
        for &v in &quantiles {
            let (cx, cy) = ((px_of(v) / 2) as usize, (py_of(v) / 4) as usize);
            assert!(
                grid.cell_char(cx, cy).is_some(),
                "no dot at tick cell ({cx},{cy}) for value {v}"
            );
        }
    }

    #[test]
    fn axis_ticks_anchor_to_data() {
        // default view (viewport = padded data range) shows all five quantiles
        let t = axis_ticks(0.0, 100.0, -2.0, 102.0);
        assert_eq!(t, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        // zoomed in: only quantiles inside the window remain
        assert_eq!(axis_ticks(0.0, 100.0, 40.0, 60.0), vec![50.0]);
        // zoomed out keeps ticks anchored to the data range, none beyond it
        let out = axis_ticks(0.0, 100.0, -50.0, 150.0);
        assert_eq!(out, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        // panned off the data entirely: no ticks
        assert!(axis_ticks(0.0, 100.0, 200.0, 300.0).is_empty());
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

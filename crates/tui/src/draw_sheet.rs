//! Table renderer: column layout, header, rows, colorizer precedence.

use rustidata_core::options::Options;
use rustidata_core::sheet::Sheet;
use rustidata_core::value::Value;

use crate::cliptext::{dispwidth, fit};
use crate::screen::ScreenBuffer;
use crate::theme::{StyleKey, Theme};

pub struct ColLayout {
    /// Index into sheet.columns.
    pub col: usize,
    /// Index into sheet.visible_cols() (cursor space).
    pub vis_idx: usize,
    pub x: u16,
    pub w: u16,
}

pub const MIN_COL_WIDTH: u16 = 4;

/// Compute layout for the visible viewport. Key columns first, then non-key
/// columns starting at `sheet.view.left_col`.
pub fn calc_layout(sheet: &Sheet, opts: &Options, width: u16, nscreen: u64) -> Vec<ColLayout> {
    let vis = sheet.visible_cols();
    let nkeys = sheet.n_key_cols();
    let mut layout = Vec::new();
    let mut x: u16 = 0;
    let sep_w = opts.col_gap;

    let mut place = |ci: usize, vis_idx: usize, x: &mut u16| -> bool {
        // clamp to remaining space so an over-wide column renders truncated
        // at the screen edge instead of disappearing entirely
        let w = col_width(sheet, opts, ci, nscreen).min(width.saturating_sub(*x + sep_w));
        if w < MIN_COL_WIDTH {
            return false;
        }
        layout.push(ColLayout {
            col: ci,
            vis_idx,
            x: *x,
            w,
        });
        *x += w + sep_w;
        true
    };

    for (vi, &ci) in vis.iter().take(nkeys).enumerate() {
        if !place(ci, vi, &mut x) {
            return layout;
        }
    }
    for (vi, &ci) in vis.iter().enumerate().skip(nkeys + sheet.view.left_col) {
        if !place(ci, vi, &mut x) {
            break;
        }
    }
    layout
}

/// Width the column currently renders at (explicit width, or auto-fit sample).
pub fn col_width(sheet: &Sheet, opts: &Options, ci: usize, nscreen: u64) -> u16 {
    let col = &sheet.columns[ci];
    if let Some(w) = col.width {
        return w.max(MIN_COL_WIDTH);
    }
    max_col_width(sheet, ci, nscreen, opts.max_width)
}

/// Max display width of the header and visible-row values for column `ci`,
/// capped at `cap` (e.g. screen width for full expansion).
pub fn max_col_width(sheet: &Sheet, ci: usize, nscreen: u64, cap: u16) -> u16 {
    let col = &sheet.columns[ci];
    let mut w = dispwidth(&col.header_text());
    let top = sheet.view.top_row;
    for vi in top..(top + nscreen).min(sheet.nrows()) {
        if let Some(rid) = sheet.rowid_at(vi) {
            let disp = col.display_value(sheet, rid);
            w = w.max(dispwidth(&disp));
            if w >= cap as usize {
                break;
            }
        }
    }
    (w as u16).clamp(MIN_COL_WIDTH, cap)
}

/// Scroll left_col so the cursor column is on screen at its full width.
pub fn ensure_cursor_col_visible(sheet: &mut Sheet, opts: &Options, width: u16, nscreen: u64) {
    let nkeys = sheet.n_key_cols();
    if sheet.view.cur_col < nkeys {
        return; // key columns always visible
    }
    let rest_idx = sheet.view.cur_col - nkeys;
    if rest_idx < sheet.view.left_col {
        sheet.view.left_col = rest_idx;
        return;
    }
    // Scroll until the cursor column renders at its full width, not merely
    // appears: calc_layout clamps a rightmost column to the space before the
    // screen edge, so widening it (`>`/`_`) must pull left_col forward to make
    // room. Once the column is leftmost, more scrolling cannot widen it.
    let Some(ci) = sheet.cursor_col_index() else { return };
    let desired = col_width(sheet, opts, ci, nscreen);
    for _ in 0..256 {
        let layout = calc_layout(sheet, opts, width, nscreen);
        if layout
            .iter()
            .any(|l| l.vis_idx == sheet.view.cur_col && l.w >= desired)
        {
            return;
        }
        if sheet.view.left_col >= rest_idx {
            return;
        }
        sheet.view.left_col += 1;
    }
}

/// Draw the sheet into rows [y0, y0+height) of the buffer.
/// Returns the number of data rows that fit (for page-size computation).
pub fn draw(
    sb: &mut ScreenBuffer,
    sheet: &Sheet,
    opts: &Options,
    theme: &Theme,
    y0: u16,
    height: u16,
) -> u64 {
    if height < 2 {
        return 0;
    }
    let nscreen = (height - 1) as u64;
    let width = sb.width();
    let layout = calc_layout(sheet, opts, width, nscreen);
    let base = theme.get(StyleKey::Default);
    let sep_style = base.merge_over(&theme.get(StyleKey::ColumnSep));
    let truncator = if theme.ascii_only { "~" } else { &opts.disp_truncator };
    let col_sep = if theme.ascii_only { "|" } else { opts.disp_column_sep.as_str() };
    let key_sep = if theme.ascii_only { "!" } else { opts.disp_keycol_sep.as_str() };
    // center the 1-cell separator glyph in the col_gap-wide gutter
    let gap = opts.col_gap as usize;
    let pad_l = gap.saturating_sub(1) / 2;
    let pad_r = gap.saturating_sub(1) - pad_l;
    let pad = |sep: &str| {
        if gap == 0 {
            String::new()
        } else {
            format!("{}{}{}", " ".repeat(pad_l), sep, " ".repeat(pad_r))
        }
    };
    let col_sep = pad(col_sep);
    let key_sep = pad(key_sep);
    let nkeys = sheet.n_key_cols();

    // header
    for l in &layout {
        let col = &sheet.columns[l.col];
        let mut style = base.merge_over(&theme.get(StyleKey::Header));
        if col.is_key() {
            style = style.merge_over(&theme.get(StyleKey::KeyCol));
        }
        if l.vis_idx == sheet.view.cur_col {
            style = style.merge_over(&theme.get(StyleKey::CurrentHdr));
        }
        let text = fit(&col.header_text(), l.w as usize, truncator);
        sb.put_str(l.x, y0, &text, style);
        let sep = if l.vis_idx + 1 == nkeys { &key_sep } else { &col_sep };
        sb.put_str(l.x + l.w, y0, sep, sep_style);
    }

    // rows
    for i in 0..nscreen {
        let y = y0 + 1 + i as u16;
        let vi = sheet.view.top_row + i;
        if vi >= sheet.nrows() {
            break;
        }
        let Some(rid) = sheet.rowid_at(vi) else { break };
        let is_cur_row = vi == sheet.view.cur_row;
        let is_selected = sheet.selection.contains(rid);

        for l in &layout {
            let col = &sheet.columns[l.col];
            let typed = col.typed_value(sheet, rid);
            let disp = col.vtype.format(&typed, col.fmtstr.as_deref());

            // colorizer precedence (low to high): keycol < selected <
            // current-col < current-row < current-cell; error overrides fg
            let mut style = base;
            if col.is_key() {
                style = style.merge_over(&theme.get(StyleKey::KeyCol));
            }
            if is_selected {
                style = style.merge_over(&theme.get(StyleKey::SelectedRow));
            }
            if matches!(typed, Value::Error(_)) {
                style = style.merge_over(&theme.get(StyleKey::Error));
            }
            let is_cur_col = l.vis_idx == sheet.view.cur_col;
            if is_cur_col {
                style = style.merge_over(&theme.get(StyleKey::CurrentCol));
            }
            if is_cur_row {
                style = style.merge_over(&theme.get(StyleKey::CurrentRow));
            }
            if is_cur_row && is_cur_col {
                style = style.merge_over(&theme.get(StyleKey::CurrentCell));
            }

            let cell = if typed.is_null() && !opts.disp_null.is_empty() {
                fit(&opts.disp_null, l.w as usize, truncator)
            } else {
                fit(&disp, l.w as usize, truncator)
            };
            sb.put_str(l.x, y, &cell, style);
            let sep = if l.vis_idx + 1 == nkeys { &key_sep } else { &col_sep };
            sb.put_str(l.x + l.w, y, sep, sep_style);
        }
    }
    nscreen
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
                vec![Value::Str("alice".into()), Value::Int(30)],
                vec![Value::Str("bob".into()), Value::Int(25)],
            ],
        );
        Sheet::new(
            "t",
            Arc::new(store),
            vec![Column::source("name", 0), Column::source("age", 1)],
        )
    }

    #[test]
    fn renders_table() {
        use crate::theme::Style;
        let mut sb = ScreenBuffer::new(30, 5, Style::empty());
        sb.clear();
        let s = sheet();
        let theme = crate::theme::builtin_themes().remove(0);
        draw(&mut sb, &s, &Options::default(), &theme, 0, 5);
        let text = sb.to_text();
        assert!(text.contains("name"), "got: {}", text);
        assert!(text.contains("alice"));
        assert!(text.contains("30"));
    }

    #[test]
    fn rightmost_column_expands_to_full_width() {
        // Regression: with the cursor on the rightmost column, widening it
        // must scroll left_col so the column gets its full requested width
        // instead of staying clamped at the screen edge.
        let store = ColumnarStore::from_rows(
            3,
            vec![vec![Value::Int(1), Value::Int(2), Value::Int(3)]],
        );
        let mut s = Sheet::new(
            "t",
            Arc::new(store),
            vec![
                Column::source("a", 0),
                Column::source("b", 1),
                Column::source("c", 2),
            ],
        );
        s.columns[0].width = Some(8);
        s.columns[1].width = Some(8);
        s.columns[2].width = Some(20); // wider than the space left at its x
        s.view.cur_col = 2;
        let opts = Options::default();

        // before scrolling, layout clamps c below its requested width
        let clamped = calc_layout(&s, &opts, 30, 5)
            .iter()
            .find(|l| l.vis_idx == 2)
            .map(|l| l.w)
            .unwrap_or(0);
        assert!(clamped < 20, "test premise: c starts clamped, got {clamped}");

        ensure_cursor_col_visible(&mut s, &opts, 30, 5);
        let layout = calc_layout(&s, &opts, 30, 5);
        let c = layout
            .iter()
            .find(|l| l.vis_idx == 2)
            .expect("cursor column on screen");
        assert_eq!(c.w, 20, "rightmost column should render at full width");
    }

    #[test]
    fn layout_clips_to_width() {
        let s = sheet();
        let layout = calc_layout(&s, &Options::default(), 8, 5);
        assert!(layout.len() <= 2);
        for l in &layout {
            assert!(l.x + l.w <= 8);
        }
    }
}

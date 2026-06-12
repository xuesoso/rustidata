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

const MIN_COL_WIDTH: u16 = 4;

/// Compute layout for the visible viewport. Key columns first, then non-key
/// columns starting at `sheet.view.left_col`.
pub fn calc_layout(sheet: &Sheet, opts: &Options, width: u16, nscreen: u64) -> Vec<ColLayout> {
    let vis = sheet.visible_cols();
    let nkeys = sheet.n_key_cols();
    let mut layout = Vec::new();
    let mut x: u16 = 0;
    let sep_w = 1u16;

    let mut place = |ci: usize, vis_idx: usize, x: &mut u16| -> bool {
        let w = col_width(sheet, opts, ci, nscreen);
        if *x + w + sep_w > width {
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

fn col_width(sheet: &Sheet, opts: &Options, ci: usize, nscreen: u64) -> u16 {
    let col = &sheet.columns[ci];
    if let Some(w) = col.width {
        return w.max(MIN_COL_WIDTH);
    }
    // auto: sample header + visible rows
    let mut w = dispwidth(&col.header_text());
    let top = sheet.view.top_row;
    for vi in top..(top + nscreen).min(sheet.nrows()) {
        if let Some(rid) = sheet.rowid_at(vi) {
            let disp = col.display_value(sheet, rid);
            w = w.max(dispwidth(&disp));
        }
    }
    (w as u16).clamp(MIN_COL_WIDTH, opts.max_width)
}

/// Scroll left_col so the cursor column is on screen.
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
    for _ in 0..256 {
        let layout = calc_layout(sheet, opts, width, nscreen);
        if layout.iter().any(|l| l.vis_idx == sheet.view.cur_col) {
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
        let sep = if l.vis_idx + 1 == nkeys { key_sep } else { col_sep };
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
            let sep = if l.vis_idx + 1 == nkeys { key_sep } else { col_sep };
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
    fn layout_clips_to_width() {
        let s = sheet();
        let layout = calc_layout(&s, &Options::default(), 8, 5);
        assert!(layout.len() <= 2);
        for l in &layout {
            assert!(l.x + l.w <= 8);
        }
    }
}

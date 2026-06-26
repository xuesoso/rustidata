//! Cell-value pager (visidata-ish `z Enter`): show one cell's full value,
//! word-wrapped to the screen width and vertically scrollable, so values wider
//! than the terminal stay fully readable. Pushed onto the sheet stack as a
//! `Text` sheet; `q` (or `Esc`) closes it like any other dive.

use std::collections::HashMap;
use std::sync::Arc;

use rustidata_core::sheet::{Sheet, SheetId, SheetKind};
use rustidata_core::store::{ColumnarStore, RowId};
use rustidata_core::value::Value;

use crate::app::App;
use crate::cliptext::{dispwidth, fit};
use crate::screen::ScreenBuffer;
use crate::theme::{StyleKey, Theme};

pub struct TextView {
    pub title: String,
    pub body: String,
    /// Top wrapped-line offset.
    pub scroll: usize,
    /// Visible body rows and total wrapped lines, refreshed each render so key
    /// handling can page and clamp without re-wrapping.
    pub page: usize,
    pub total_lines: usize,
    /// Originating cell, so `e` can write edits back through the source sheet.
    pub src_sheet: SheetId,
    pub src_rid: RowId,
    pub src_ci: usize,
}

/// Per-app text-pager store (sheet id -> view); lives in App.
pub type TextStore = HashMap<SheetId, TextView>;

/// `z Enter`: open the cursor cell's full value in the pager.
pub fn open_cell_pager(app: &mut App) -> Result<(), anyhow::Error> {
    let Some(s) = app.sheet() else { return Ok(()) };
    let Some(ci) = s.cursor_col_index() else { return Ok(()) };
    let Some(rid) = s.cursor_rowid() else {
        app.info("no cell under cursor");
        return Ok(());
    };
    let title = s.columns[ci].name.clone();
    let body = s.columns[ci].typed_value(s, rid).to_display().to_string();
    let parent_id = s.id;

    // placeholder backing sheet so the stack/q semantics work (like graphs)
    let store = ColumnarStore::from_rows(1, Vec::<Vec<Value>>::new());
    let mut sheet = Sheet::new(
        format!("{} (cell)", title),
        Arc::new(store),
        vec![rustidata_core::column::Column::source("text", 0)],
    );
    sheet.kind = SheetKind::Text;
    sheet.parent = Some(parent_id);
    let tid = sheet.id;
    app.texts.insert(
        tid,
        TextView {
            title,
            body,
            scroll: 0,
            page: 1,
            total_lines: 0,
            src_sheet: parent_id,
            src_rid: rid,
            src_ci: ci,
        },
    );
    app.push_sheet(sheet);
    app.info("cell: j/k scroll, PgUp/PgDn page, g/G top/bottom, e edit, q close");
    Ok(())
}

/// Pager-specific keys; returns true if handled.
pub fn handle_text_key(app: &mut App, ks: &str) -> bool {
    let Some(s) = app.sheet() else { return false };
    if s.kind != SheetKind::Text {
        return false;
    }
    let id = s.id;
    // `e` opens the input editor pre-filled with the value; on submit it writes
    // back to the originating cell (see finish_input). Handle it before the
    // mutable texts borrow, since start_input mutably borrows app.
    if ks == "e" {
        let Some(initial) = app.texts.get(&id).map(|tv| tv.body.clone()) else {
            return false;
        };
        app.start_input(crate::app::InputPurpose::EditPagerCell, "edit: ", initial, vec![]);
        return true;
    }
    let Some(tv) = app.texts.get_mut(&id) else { return false };
    let page = tv.page.max(1);
    let max_scroll = tv.total_lines.saturating_sub(tv.page);
    match ks {
        "j" | "Down" => tv.scroll = (tv.scroll + 1).min(max_scroll),
        "k" | "Up" => tv.scroll = tv.scroll.saturating_sub(1),
        "PgDn" | "Space" => tv.scroll = (tv.scroll + page).min(max_scroll),
        "PgUp" => tv.scroll = tv.scroll.saturating_sub(page),
        "g" | "Home" => tv.scroll = 0,
        "G" | "End" => tv.scroll = max_scroll,
        "q" | "Esc" => {
            app.texts.remove(&id);
            app.pop_sheet();
        }
        _ => return false,
    }
    app.dirty = true;
    true
}

pub fn draw_text_pager(
    sb: &mut ScreenBuffer,
    theme: &Theme,
    tv: &mut TextView,
    y0: u16,
    height: u16,
) {
    let base = theme.get(StyleKey::Default);
    let accent = base.merge_over(&theme.get(StyleKey::Status));
    let width = sb.width() as usize;
    if height < 3 || width < 2 {
        return;
    }

    let lines = wrap(&tv.body, width);
    let body_rows = (height as usize) - 3; // header, separator, footer
    tv.page = body_rows.max(1);
    tv.total_lines = lines.len();
    let max_scroll = lines.len().saturating_sub(body_rows);
    if tv.scroll > max_scroll {
        tv.scroll = max_scroll;
    }

    // header + separator
    sb.put_str(0, y0, &fit(&format!("value: {}", tv.title), width, "…"), accent);
    sb.put_str(0, y0 + 1, &"─".repeat(width), accent);
    // body
    for (i, line) in lines.iter().skip(tv.scroll).take(body_rows).enumerate() {
        sb.put_str(0, y0 + 2 + i as u16, line, base);
    }
    // footer with scroll position
    let shown = (tv.scroll + body_rows).min(lines.len());
    let pct = if lines.is_empty() { 100 } else { shown * 100 / lines.len() };
    let footer = format!(
        "  j/k scroll · PgUp/PgDn page · g/G top/bottom · q close   [{}%]",
        pct.min(100)
    );
    sb.put_str(0, y0 + height - 1, &fit(&footer, width, "…"), accent);
}

/// Word-wrap `body` to `width` display columns, honoring existing newlines and
/// hard-splitting tokens (e.g. long JSON) that are themselves wider than width.
fn wrap(body: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in body.split('\n') {
        let mut line = String::new();
        let mut line_w = 0usize;
        for word in para.split_inclusive(' ') {
            let ww = dispwidth(word);
            if line_w > 0 && line_w + ww > width {
                out.push(std::mem::take(&mut line));
                line_w = 0;
            }
            if ww > width {
                // token longer than a full line: emit char by char
                for ch in word.chars() {
                    let cw = dispwidth(ch.encode_utf8(&mut [0u8; 4]));
                    if line_w + cw > width {
                        out.push(std::mem::take(&mut line));
                        line_w = 0;
                    }
                    line.push(ch);
                    line_w += cw;
                }
            } else {
                line.push_str(word);
                line_w += ww;
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_respects_width_and_newlines() {
        let lines = wrap("hello world foo", 7);
        assert!(lines.iter().all(|l| dispwidth(l) <= 7));
        assert!(lines.len() >= 2);
        // explicit newline forces a break
        assert_eq!(wrap("a\nb", 80), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn wrap_hard_splits_long_token() {
        let long = "x".repeat(20);
        let lines = wrap(&long, 5);
        assert!(lines.iter().all(|l| dispwidth(l) <= 5));
        assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), 20);
    }
}

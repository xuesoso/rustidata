//! Double-buffered screen with diffed flush.
//!
//! We deliberately use crossterm directly (no ratatui): the only widgets are
//! a table and a braille canvas, and owning the cell grid gives exact control
//! over wide-char occupancy and style merging.

use std::io::Write;

use crossterm::style::{Attribute, Color};
use crossterm::{cursor, queue, style, terminal};
use unicode_width::UnicodeWidthStr;

use crate::theme::Style;

#[derive(Clone, PartialEq)]
struct Cell {
    /// One grapheme cluster ("" for the shadow cell after a wide char).
    sym: compact_str::CompactString,
    style: Style,
}

impl Cell {
    fn blank(style: Style) -> Self {
        Cell {
            sym: " ".into(),
            style,
        }
    }
}

pub struct ScreenBuffer {
    w: u16,
    h: u16,
    front: Vec<Cell>,
    back: Vec<Cell>,
    default_style: Style,
    force_full: bool,
}

impl ScreenBuffer {
    pub fn new(w: u16, h: u16, default_style: Style) -> Self {
        let n = w as usize * h as usize;
        ScreenBuffer {
            w,
            h,
            front: vec![Cell::blank(default_style); n],
            back: vec![Cell::blank(default_style); n],
            default_style,
            force_full: true,
        }
    }

    pub fn width(&self) -> u16 {
        self.w
    }

    pub fn height(&self) -> u16 {
        self.h
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.w = w;
        self.h = h;
        let n = w as usize * h as usize;
        self.front = vec![Cell::blank(self.default_style); n];
        self.back = vec![Cell::blank(self.default_style); n];
        self.force_full = true;
    }

    pub fn set_default_style(&mut self, style: Style) {
        self.default_style = style;
        self.force_full = true;
    }

    /// Clear the back buffer to the default style.
    pub fn clear(&mut self) {
        let blank = Cell::blank(self.default_style);
        for c in self.back.iter_mut() {
            *c = blank.clone();
        }
    }

    fn idx(&self, x: u16, y: u16) -> usize {
        y as usize * self.w as usize + x as usize
    }

    /// Write a string at (x, y), clipping at the right edge. Returns the
    /// number of cells consumed.
    pub fn put_str(&mut self, x: u16, y: u16, s: &str, style: Style) -> u16 {
        if y >= self.h || x >= self.w {
            return 0;
        }
        let mut cx = x;
        for g in unicode_segmentation::UnicodeSegmentation::graphemes(s, true) {
            let gw = UnicodeWidthStr::width(g).max(1) as u16;
            if cx + gw > self.w {
                break;
            }
            let i = self.idx(cx, y);
            self.back[i] = Cell {
                sym: g.into(),
                style,
            };
            // shadow cells for wide chars
            for k in 1..gw {
                let j = self.idx(cx + k, y);
                self.back[j] = Cell {
                    sym: "".into(),
                    style,
                };
            }
            cx += gw;
        }
        cx - x
    }

    /// Merge a style over an existing region (e.g. cursor-row highlight).
    pub fn style_region(&mut self, x: u16, y: u16, w: u16, style: Style) {
        if y >= self.h {
            return;
        }
        let end = (x + w).min(self.w);
        for cx in x..end {
            let i = self.idx(cx, y);
            self.back[i].style = self.back[i].style.merge_over(&style);
        }
    }

    /// Diff back vs front, emit minimal updates, swap.
    pub fn flush(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        queue!(out, terminal::BeginSynchronizedUpdate)?;
        let mut last_style: Option<Style> = None;
        let mut cursor_at: Option<(u16, u16)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                let i = self.idx(x, y);
                if !self.force_full && self.back[i] == self.front[i] {
                    continue;
                }
                if self.back[i].sym.is_empty() {
                    continue; // shadow cell: emitted with its wide char
                }
                if cursor_at != Some((x, y)) {
                    queue!(out, cursor::MoveTo(x, y))?;
                }
                if last_style != Some(self.back[i].style) {
                    apply_style(out, &self.back[i].style)?;
                    last_style = Some(self.back[i].style);
                }
                queue!(out, style::Print(self.back[i].sym.as_str()))?;
                let adv = UnicodeWidthStr::width(self.back[i].sym.as_str()).max(1) as u16;
                cursor_at = Some((x + adv, y));
            }
        }
        queue!(out, terminal::EndSynchronizedUpdate)?;
        out.flush()?;
        std::mem::swap(&mut self.front, &mut self.back);
        self.force_full = false;
        Ok(())
    }

    /// Render the back buffer to plain text (for snapshot tests).
    #[allow(dead_code)]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for y in 0..self.h {
            let mut line = String::new();
            for x in 0..self.w {
                let i = self.idx(x, y);
                line.push_str(&self.back[i].sym);
            }
            s.push_str(line.trim_end());
            s.push('\n');
        }
        s
    }
}

fn apply_style(out: &mut impl Write, style: &Style) -> std::io::Result<()> {
    queue!(
        out,
        style::SetAttribute(Attribute::Reset),
        style::SetForegroundColor(style.fg.unwrap_or(Color::Reset)),
        style::SetBackgroundColor(style.bg.unwrap_or(Color::Reset)),
    )?;
    for attr in [
        Attribute::Bold,
        Attribute::Underlined,
        Attribute::Reverse,
        Attribute::Dim,
        Attribute::Italic,
        Attribute::SlowBlink,
    ] {
        if style.attrs.has(attr) {
            queue!(out, style::SetAttribute(attr))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_render() {
        let mut sb = ScreenBuffer::new(10, 2, Style::empty());
        sb.clear();
        sb.put_str(0, 0, "hello", Style::empty());
        sb.put_str(2, 1, "world", Style::empty());
        assert_eq!(sb.to_text(), "hello\n  world\n");
    }

    #[test]
    fn wide_chars_occupy_two_cells() {
        let mut sb = ScreenBuffer::new(6, 1, Style::empty());
        sb.clear();
        let consumed = sb.put_str(0, 0, "日本", Style::empty());
        assert_eq!(consumed, 4);
        assert_eq!(sb.to_text(), "日本\n");
    }

    #[test]
    fn clips_at_right_edge() {
        let mut sb = ScreenBuffer::new(3, 1, Style::empty());
        sb.clear();
        sb.put_str(0, 0, "abcdef", Style::empty());
        assert_eq!(sb.to_text(), "abc\n");
    }

    #[test]
    fn wide_char_not_split_at_edge() {
        let mut sb = ScreenBuffer::new(3, 1, Style::empty());
        sb.clear();
        sb.put_str(0, 0, "a日本", Style::empty());
        // 日 fits (cells 1-2), 本 would straddle: dropped
        assert_eq!(sb.to_text(), "a日\n");
    }

    #[test]
    fn diff_flush_emits_changes() {
        let mut sb = ScreenBuffer::new(5, 1, Style::empty());
        sb.clear();
        sb.put_str(0, 0, "aaaaa", Style::empty());
        let mut out: Vec<u8> = Vec::new();
        sb.flush(&mut out).unwrap();
        assert!(!out.is_empty());

        // unchanged content -> nearly empty diff
        sb.clear();
        sb.put_str(0, 0, "aaaaa", Style::empty());
        let mut out2: Vec<u8> = Vec::new();
        sb.flush(&mut out2).unwrap();
        assert!(out2.len() < out.len());
    }
}

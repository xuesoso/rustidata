//! Display-width-aware text clipping (CJK wide chars, combining marks).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Display width of a string in terminal cells.
pub fn dispwidth(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Clip `s` to at most `width` cells, appending `truncator` if clipped.
/// Returns (clipped string, its display width). A wide char straddling the
/// boundary is dropped and padded with a space.
pub fn clipstr(s: &str, width: usize, truncator: &str) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let full = dispwidth(s);
    if full <= width {
        return (sanitize(s), full);
    }
    let tw = dispwidth(truncator);
    let budget = width.saturating_sub(tw);
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let gw = UnicodeWidthStr::width(g);
        if used + gw > budget {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out.push_str(truncator);
    used += tw;
    if used < width.min(full) {
        // wide char didn't fit flush; pad
        out.push(' ');
        used += 1;
    }
    (sanitize(&out), used)
}

/// Pad or clip to exactly `width` cells.
pub fn fit(s: &str, width: usize, truncator: &str) -> String {
    let (mut out, used) = clipstr(s, width, truncator);
    for _ in used..width {
        out.push(' ');
    }
    out
}

/// Replace control characters with printable escapes.
fn sanitize(s: &str) -> String {
    if s.chars().all(|c| !c.is_control()) {
        return s.to_string();
    }
    s.chars()
        .map(|c| match c {
            '\n' => '␊',
            '\t' => '␉',
            '\r' => '␍',
            c if c.is_control() => '·',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_clip() {
        assert_eq!(clipstr("hello", 10, "…"), ("hello".into(), 5));
        assert_eq!(clipstr("hello world", 8, "…"), ("hello w…".into(), 8));
        assert_eq!(clipstr("hello", 0, "…"), ("".into(), 0));
    }

    #[test]
    fn cjk_widths() {
        assert_eq!(dispwidth("日本語"), 6);
        let (s, w) = clipstr("日本語テスト", 7, "…");
        assert!(w <= 7);
        assert!(s.ends_with('…') || s.ends_with(' '));
    }

    #[test]
    fn combining_marks() {
        // e + combining acute = width 1
        assert_eq!(dispwidth("e\u{0301}"), 1);
    }

    #[test]
    fn exact_fit_pads() {
        assert_eq!(fit("ab", 4, "…"), "ab  ");
        assert_eq!(fit("abcdef", 4, "…"), "abc…");
    }

    #[test]
    fn control_chars_sanitized() {
        let (s, _) = clipstr("a\tb\nc", 10, "…");
        assert_eq!(s, "a␉b␊c");
    }
}

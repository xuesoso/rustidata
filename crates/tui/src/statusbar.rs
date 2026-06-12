//! Status bars: bottom-left sheet info, bottom-right progress/keystrokes,
//! top line reserved for the menu bar.

use rustidata_core::jobs::Severity;
use rustidata_core::sheet::Sheet;

use crate::cliptext::fit;
use crate::screen::ScreenBuffer;
use crate::theme::{StyleKey, Theme};

pub fn draw_status(
    sb: &mut ScreenBuffer,
    sheet: &Sheet,
    theme: &Theme,
    msg: Option<&(Severity, String)>,
    prefix: &str,
    y: u16,
) {
    let width = sb.width() as usize;
    let base = theme.get(StyleKey::Default).merge_over(&theme.get(StyleKey::Status));

    // left: sheet name | nrows | selection | sort
    let mut left = format!(" {} ", sheet.name);
    left.push_str(&format!("{} rows", sheet.nrows()));
    if sheet.n_selected() > 0 {
        left.push_str(&format!("  {} selected", sheet.n_selected()));
    }
    if !sheet.sort_spec.is_empty() {
        let spec: Vec<String> = sheet
            .sort_spec
            .iter()
            .map(|&(c, desc)| {
                format!(
                    "{}{}",
                    sheet.columns[c].name,
                    if desc { "-" } else { "+" }
                )
            })
            .collect();
        left.push_str(&format!("  sort:{}", spec.join(",")));
    }

    // right: pending prefix, loading progress
    let mut right = String::new();
    if !prefix.is_empty() {
        right.push_str(&format!("{}… ", prefix));
    }
    if let Some(p) = sheet.loading() {
        match p.pct() {
            Some(pct) => right.push_str(&format!("{} {}% ", p.gerund, pct)),
            None => right.push_str(&format!("{} {} ", p.gerund, p.done)),
        }
    }

    let mut line = fit(&left, width, "…");
    let rw = crate::cliptext::dispwidth(&right);
    if rw > 0 && rw < width {
        line.truncate_floor(width - rw);
        line.push_str(&right);
    }
    sb.put_str(0, y, &line, base);

    // transient message overlays the left side
    if let Some((sev, text)) = msg {
        let style = match sev {
            Severity::Error => base.merge_over(&theme.get(StyleKey::Error)),
            Severity::Warning => base.merge_over(&theme.get(StyleKey::Warning)),
            Severity::Info => base,
        };
        sb.put_str(0, y, &fit(&format!(" {} ", text), width.min(text.len() + 2), "…"), style);
    }
}

trait TruncFloor {
    fn truncate_floor(&mut self, n: usize);
}

impl TruncFloor for String {
    /// Truncate to at most n bytes without splitting a char.
    fn truncate_floor(&mut self, n: usize) {
        let mut k = n.min(self.len());
        while k > 0 && !self.is_char_boundary(k) {
            k -= 1;
        }
        self.truncate(k);
    }
}

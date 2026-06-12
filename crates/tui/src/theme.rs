//! Themes: named sets of styles for every UI slot, switchable at runtime and
//! persisted to the user config. (VisiData has no theme switcher — this is a
//! rustidata addition; slot names follow visidata's color_* options.)

use crossterm::style::{Attribute, Attributes, Color};

/// Every styleable UI slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleKey {
    Default,
    Header,
    CurrentRow,
    CurrentCol,
    CurrentHdr,
    CurrentCell,
    KeyCol,
    SelectedRow,
    ColumnSep,
    Error,
    Warning,
    Note,
    Status,
    TopStatus,
    Menu,
    MenuActive,
    EditCell,
    GraphAxis,
    Histogram,
    Loading,
}

impl StyleKey {
    pub fn all() -> &'static [StyleKey] {
        use StyleKey::*;
        &[
            Default, Header, CurrentRow, CurrentCol, CurrentHdr, CurrentCell, KeyCol,
            SelectedRow, ColumnSep, Error, Warning, Note, Status, TopStatus, Menu,
            MenuActive, EditCell, GraphAxis, Histogram, Loading,
        ]
    }

    /// Config name (matches visidata's option naming where possible).
    pub fn name(&self) -> &'static str {
        use StyleKey::*;
        match self {
            Default => "default",
            Header => "header",
            CurrentRow => "current_row",
            CurrentCol => "current_col",
            CurrentHdr => "current_hdr",
            CurrentCell => "current_cell",
            KeyCol => "key_col",
            SelectedRow => "selected_row",
            ColumnSep => "column_sep",
            Error => "error",
            Warning => "warning",
            Note => "note",
            Status => "status",
            TopStatus => "top_status",
            Menu => "menu",
            MenuActive => "menu_active",
            EditCell => "edit_cell",
            GraphAxis => "graph_axis",
            Histogram => "histogram",
            Loading => "loading",
        }
    }

    pub fn from_name(s: &str) -> Option<StyleKey> {
        StyleKey::all().iter().copied().find(|k| k.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub attrs: Attributes,
}

impl Style {
    pub const fn empty() -> Self {
        Style {
            fg: None,
            bg: None,
            attrs: Attributes::none(),
        }
    }

    /// Merge another style on top of this one (other wins where set).
    pub fn merge_over(&self, other: &Style) -> Style {
        Style {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            attrs: self.attrs | other.attrs,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Style::empty()
    }
}

/// Parse a visidata-style color string: "bold black on 110", "red", "reverse",
/// "underline white on 238".
pub fn parse_style(s: &str) -> Style {
    let mut style = Style::empty();
    let mut on_bg = false;
    for word in s.split_whitespace() {
        match word.to_ascii_lowercase().as_str() {
            "on" => on_bg = true,
            "bold" => style.attrs.set(Attribute::Bold),
            "underline" => style.attrs.set(Attribute::Underlined),
            "reverse" => style.attrs.set(Attribute::Reverse),
            "dim" => style.attrs.set(Attribute::Dim),
            "italic" => style.attrs.set(Attribute::Italic),
            "blink" => style.attrs.set(Attribute::SlowBlink),
            w => {
                if let Some(c) = parse_color(w) {
                    if on_bg {
                        style.bg = Some(c);
                    } else {
                        style.fg = Some(c);
                    }
                }
            }
        }
    }
    style
}

fn parse_color(w: &str) -> Option<Color> {
    if let Ok(n) = w.parse::<u8>() {
        return Some(Color::AnsiValue(n));
    }
    Some(match w {
        "black" => Color::Black,
        "red" => Color::DarkRed,
        "green" => Color::DarkGreen,
        "yellow" => Color::DarkYellow,
        "blue" => Color::DarkBlue,
        "magenta" => Color::DarkMagenta,
        "cyan" => Color::DarkCyan,
        "white" => Color::White,
        "grey" | "gray" => Color::Grey,
        "brightred" => Color::Red,
        "brightgreen" => Color::Green,
        "brightyellow" => Color::Yellow,
        "brightblue" => Color::Blue,
        "brightmagenta" => Color::Magenta,
        "brightcyan" => Color::Cyan,
        _ => return None,
    })
}

#[derive(Clone)]
pub struct Theme {
    pub name: &'static str,
    slots: Vec<(StyleKey, Style)>,
    /// ASCII-only glyphs (for the `ascii` theme / dumb terminals).
    pub ascii_only: bool,
}

impl Theme {
    pub fn get(&self, key: StyleKey) -> Style {
        self.slots
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, s)| *s)
            .unwrap_or_else(Style::empty)
    }

    pub fn set(&mut self, key: StyleKey, style: Style) {
        match self.slots.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = style,
            None => self.slots.push((key, style)),
        }
    }
}

fn theme(name: &'static str, ascii_only: bool, defs: &[(StyleKey, &str)]) -> Theme {
    Theme {
        name,
        ascii_only,
        slots: defs.iter().map(|(k, s)| (*k, parse_style(s))).collect(),
    }
}

/// The built-in themes, in cycle order.
pub fn builtin_themes() -> Vec<Theme> {
    use StyleKey::*;
    vec![
        theme(
            "default-dark",
            false,
            &[
                (Default, ""),
                (Header, "bold underline"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse underline"),
                (CurrentCell, "reverse bold"),
                (KeyCol, "brightyellow"),
                (SelectedRow, "brightgreen"),
                (ColumnSep, "240"),
                (Error, "brightred"),
                (Warning, "brightyellow"),
                (Note, "brightblue"),
                (Status, "bold white on 238"),
                (TopStatus, "white on 238"),
                (Menu, "black on 110"),
                (MenuActive, "bold white on 24"),
                (EditCell, "white on 238"),
                (GraphAxis, "bold"),
                (Histogram, "blue"),
                (Loading, "brightmagenta"),
            ],
        ),
        theme(
            "light",
            false,
            &[
                (Default, "black on white"),
                (Header, "bold underline blue on white"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse"),
                (CurrentCell, "reverse bold"),
                (KeyCol, "blue on white"),
                (SelectedRow, "green on white"),
                (ColumnSep, "250"),
                (Error, "red on white"),
                (Warning, "yellow on white"),
                (Note, "blue on white"),
                (Status, "bold white on 25"),
                (TopStatus, "white on 25"),
                (Menu, "black on 152"),
                (MenuActive, "bold white on 25"),
                (EditCell, "black on 254"),
                (GraphAxis, "bold black on white"),
                (Histogram, "blue on white"),
                (Loading, "magenta on white"),
            ],
        ),
        theme(
            "monochrome",
            false,
            &[
                (Default, ""),
                (Header, "bold underline"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse"),
                (CurrentCell, "reverse"),
                (KeyCol, "bold"),
                (SelectedRow, "underline"),
                (ColumnSep, "dim"),
                (Error, "reverse bold"),
                (Warning, "bold"),
                (Note, "dim"),
                (Status, "reverse"),
                (TopStatus, "reverse dim"),
                (Menu, "reverse"),
                (MenuActive, "bold"),
                (EditCell, "underline"),
                (GraphAxis, "bold"),
                (Histogram, "bold"),
                (Loading, "bold"),
            ],
        ),
        theme(
            "ascii",
            true,
            &[
                (Default, ""),
                (Header, "bold"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse"),
                (CurrentCell, "reverse"),
                (KeyCol, "bold"),
                (SelectedRow, "underline"),
                (ColumnSep, "dim"),
                (Error, "reverse"),
                (Warning, "bold"),
                (Note, "dim"),
                (Status, "reverse"),
                (TopStatus, "reverse"),
                (Menu, "reverse"),
                (MenuActive, "bold"),
                (EditCell, "underline"),
                (GraphAxis, "bold"),
                (Histogram, "bold"),
                (Loading, "bold"),
            ],
        ),
        theme(
            "solarized-dark",
            false,
            &[
                (Default, "247 on 235"),
                (Header, "bold 136 on 235"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse 136"),
                (CurrentCell, "reverse bold"),
                (KeyCol, "33 on 235"),
                (SelectedRow, "64 on 235"),
                (ColumnSep, "240"),
                (Error, "160 on 235"),
                (Warning, "136 on 235"),
                (Note, "37 on 235"),
                (Status, "bold 254 on 240"),
                (TopStatus, "245 on 236"),
                (Menu, "235 on 244"),
                (MenuActive, "bold 254 on 33"),
                (EditCell, "254 on 240"),
                (GraphAxis, "bold 136"),
                (Histogram, "33"),
                (Loading, "125 on 235"),
            ],
        ),
        theme(
            "gruvbox",
            false,
            &[
                (Default, "223 on 235"),
                (Header, "bold 214 on 235"),
                (CurrentRow, "reverse"),
                (CurrentCol, "bold"),
                (CurrentHdr, "bold reverse 214"),
                (CurrentCell, "reverse bold"),
                (KeyCol, "109 on 235"),
                (SelectedRow, "142 on 235"),
                (ColumnSep, "241"),
                (Error, "167 on 235"),
                (Warning, "214 on 235"),
                (Note, "109 on 235"),
                (Status, "bold 235 on 246"),
                (TopStatus, "246 on 237"),
                (Menu, "235 on 109"),
                (MenuActive, "bold 223 on 66"),
                (EditCell, "223 on 239"),
                (GraphAxis, "bold 214"),
                (Histogram, "109"),
                (Loading, "175 on 235"),
            ],
        ),
    ]
}

pub fn theme_by_name(name: &str) -> Option<Theme> {
    builtin_themes().into_iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_visidata_style_strings() {
        let s = parse_style("bold black on 110");
        assert_eq!(s.fg, Some(Color::Black));
        assert_eq!(s.bg, Some(Color::AnsiValue(110)));
        assert!(s.attrs.has(Attribute::Bold));

        let s = parse_style("reverse");
        assert_eq!(s.fg, None);
        assert!(s.attrs.has(Attribute::Reverse));
    }

    #[test]
    fn all_builtins_resolve() {
        for t in builtin_themes() {
            for k in StyleKey::all() {
                let _ = t.get(*k);
            }
        }
        assert!(theme_by_name("gruvbox").is_some());
        assert!(theme_by_name("nonexistent").is_none());
    }

    #[test]
    fn merge_semantics() {
        let base = parse_style("white on black");
        let over = parse_style("bold red");
        let m = base.merge_over(&over);
        assert_eq!(m.fg, Some(Color::DarkRed));
        assert_eq!(m.bg, Some(Color::Black));
        assert!(m.attrs.has(Attribute::Bold));
    }
}

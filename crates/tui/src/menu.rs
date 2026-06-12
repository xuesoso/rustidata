//! Menu bar (visidata-style): top-line menus generated from the command
//! table, navigable with arrows, executes by longname. Also the centered
//! chooser overlay used by theme/join/aggregator pickers.

use rustidata_core::jobs::Severity;

use crate::app::{App, Mode};
use crate::cliptext::fit;
use crate::screen::ScreenBuffer;
use crate::theme::{StyleKey, Theme};

pub struct MenuItem {
    pub label: &'static str,
    pub longname: &'static str,
}

pub struct Menu {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

pub static MENUS: &[Menu] = &[
    Menu {
        title: "File",
        items: &[
            MenuItem { label: "Open...", longname: "open-file" },
            MenuItem { label: "Save as...", longname: "save-sheet" },
            MenuItem { label: "Reload", longname: "reload-sheet" },
            MenuItem { label: "Quit sheet", longname: "quit-sheet" },
            MenuItem { label: "Quit all", longname: "quit-all" },
        ],
    },
    Menu {
        title: "Edit",
        items: &[
            MenuItem { label: "Edit cell", longname: "edit-cell" },
            MenuItem { label: "Add row", longname: "add-row" },
            MenuItem { label: "Delete row", longname: "delete-row" },
            MenuItem { label: "Undo", longname: "undo-last" },
            MenuItem { label: "Redo", longname: "redo-last" },
            MenuItem { label: "Yank row", longname: "copy-row" },
            MenuItem { label: "Paste row", longname: "paste-row" },
        ],
    },
    Menu {
        title: "View",
        items: &[
            MenuItem { label: "Sort ascending", longname: "sort-asc" },
            MenuItem { label: "Sort descending", longname: "sort-desc" },
            MenuItem { label: "Search", longname: "search-col" },
            MenuItem { label: "Frequency table", longname: "open-freq" },
            MenuItem { label: "Describe", longname: "open-describe" },
            MenuItem { label: "Plot column", longname: "plot-column" },
            MenuItem { label: "Choose theme...", longname: "theme-choose" },
            MenuItem { label: "Cycle theme", longname: "theme-cycle" },
        ],
    },
    Menu {
        title: "Column",
        items: &[
            MenuItem { label: "Hide", longname: "hide-col" },
            MenuItem { label: "Toggle key", longname: "key-col" },
            MenuItem { label: "Rename", longname: "rename-col" },
            MenuItem { label: "Resize", longname: "resize-col-input" },
            MenuItem { label: "Type: string", longname: "type-string" },
            MenuItem { label: "Type: int", longname: "type-int" },
            MenuItem { label: "Type: float", longname: "type-float" },
            MenuItem { label: "Type: date", longname: "type-date" },
            MenuItem { label: "Type: currency", longname: "type-currency" },
            MenuItem { label: "Add expr column", longname: "addcol-expr" },
            MenuItem { label: "Freeze column", longname: "freeze-col" },
        ],
    },
    Menu {
        title: "Data",
        items: &[
            MenuItem { label: "Pivot", longname: "open-pivot" },
            MenuItem { label: "Melt", longname: "open-melt" },
            MenuItem { label: "Transpose", longname: "open-transpose" },
            MenuItem { label: "Unfurl column", longname: "open-unfurl" },
            MenuItem { label: "Join sheets...", longname: "open-join" },
            MenuItem { label: "Fill down", longname: "setcol-fill" },
            MenuItem { label: "Freeze sheet", longname: "freeze-sheet" },
            MenuItem { label: "Select all", longname: "select-all" },
            MenuItem { label: "Unselect all", longname: "unselect-all" },
        ],
    },
    Menu {
        title: "Meta",
        items: &[
            MenuItem { label: "Sheets", longname: "sheets-sheet" },
            MenuItem { label: "Columns", longname: "columns-sheet" },
            MenuItem { label: "Options", longname: "options-sheet" },
            MenuItem { label: "Help / commands", longname: "help-commands" },
        ],
    },
];

pub struct MenuState {
    pub menu_idx: usize,
    pub item_idx: usize,
}

pub fn draw_menu_bar(sb: &mut ScreenBuffer, app: &App, y: u16) {
    let theme = &app.theme;
    let base = theme.get(StyleKey::Default).merge_over(&theme.get(StyleKey::Menu));
    let width = sb.width() as usize;
    sb.put_str(0, y, &" ".repeat(width), base);
    let mut x = 1u16;
    let open_idx = match &app.mode {
        Mode::Menu(state) => Some(state.menu_idx),
        _ => None,
    };
    for (i, m) in MENUS.iter().enumerate() {
        let style = if open_idx == Some(i) {
            base.merge_over(&theme.get(StyleKey::MenuActive))
        } else {
            base
        };
        let label = format!(" {} ", m.title);
        x += sb.put_str(x, y, &label, style) + 1;
    }
    // right side: app name + theme
    let tag = format!("rustidata [{}] ", theme.name);
    let tx = (sb.width() as usize).saturating_sub(tag.len());
    sb.put_str(tx as u16, y, &tag, base);
}

pub fn draw_open_menu(sb: &mut ScreenBuffer, app: &App, state: &MenuState) {
    let theme = &app.theme;
    let base = theme.get(StyleKey::Default).merge_over(&theme.get(StyleKey::Menu));
    let active = base.merge_over(&theme.get(StyleKey::MenuActive));

    // x offset of the open menu title
    let mut x = 1u16;
    for m in MENUS.iter().take(state.menu_idx) {
        x += m.title.len() as u16 + 3;
    }
    let menu = &MENUS[state.menu_idx];
    let w = menu
        .items
        .iter()
        .map(|i| i.label.len())
        .max()
        .unwrap_or(10)
        + 4;
    for (i, item) in menu.items.iter().enumerate() {
        let y = 1 + i as u16;
        if y >= sb.height() {
            break;
        }
        let style = if i == state.item_idx { active } else { base };
        sb.put_str(x, y, &fit(&format!(" {} ", item.label), w, "…"), style);
    }
}

pub fn handle_menu_key(app: &mut App, ks: &str) {
    let Mode::Menu(state) = &mut app.mode else { return };
    match ks {
        "Esc" | "q" | "^C" => app.mode = Mode::Normal,
        "Left" | "h" => {
            state.menu_idx = (state.menu_idx + MENUS.len() - 1) % MENUS.len();
            state.item_idx = 0;
        }
        "Right" | "l" => {
            state.menu_idx = (state.menu_idx + 1) % MENUS.len();
            state.item_idx = 0;
        }
        "Down" | "j" => {
            let n = MENUS[state.menu_idx].items.len();
            state.item_idx = (state.item_idx + 1) % n;
        }
        "Up" | "k" => {
            let n = MENUS[state.menu_idx].items.len();
            state.item_idx = (state.item_idx + n - 1) % n;
        }
        "Enter" => {
            let longname = MENUS[state.menu_idx].items[state.item_idx].longname;
            app.mode = Mode::Normal;
            if let Err(e) = crate::commands::execute(app, longname) {
                app.status(Severity::Error, format!("{}: {}", longname, e));
            }
        }
        _ => {}
    }
}

/// Centered list chooser (themes, join kinds, aggregators).
pub fn draw_chooser(
    sb: &mut ScreenBuffer,
    theme: &Theme,
    title: &str,
    items: &[String],
    selected: usize,
) {
    let base = theme.get(StyleKey::Default).merge_over(&theme.get(StyleKey::Menu));
    let active = base.merge_over(&theme.get(StyleKey::MenuActive));
    let w = items
        .iter()
        .map(|s| s.len())
        .max()
        .unwrap_or(10)
        .max(title.len())
        + 4;
    let h = items.len() + 2;
    let x0 = (sb.width() as usize).saturating_sub(w) / 2;
    let y0 = (sb.height() as usize).saturating_sub(h) / 2;

    sb.put_str(x0 as u16, y0 as u16, &fit(&format!(" {} ", title), w, "…"), active);
    for (i, item) in items.iter().enumerate() {
        let style = if i == selected { active } else { base };
        sb.put_str(
            x0 as u16,
            (y0 + 1 + i) as u16,
            &fit(&format!("  {} ", item), w, "…"),
            style,
        );
    }
    sb.put_str(
        x0 as u16,
        (y0 + 1 + items.len()) as u16,
        &fit(" Enter:choose  Esc:cancel ", w, "…"),
        base,
    );
}

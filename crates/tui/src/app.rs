//! Application state and mainloop.

use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::Receiver;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::{cursor, execute, terminal};
use rustidata_core::jobs::{JobManager, Severity, UiMsg};
use rustidata_core::options::Options;
use rustidata_core::sheet::{Sheet, SheetId};
use rustidata_core::value::Value;

use crate::clipboard::Clipboard;
use crate::input::LineEditor;
use crate::screen::ScreenBuffer;
use crate::theme::{StyleKey, Theme};

/// What an Enter on the input line means.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputPurpose {
    EditCell,
    EditPagerCell,
    EditCellsSelected,
    Search { backward: bool, all_cols: bool },
    SelectRegex { unselect: bool, all_cols: bool },
    SelectExpr { unselect: bool },
    AddExprColumn,
    RenameColumn,
    RenameSheet,
    SaveAs,
    ResizeColumn,
    ExecLongname,
    GotoRow,
    SplitRegex,
    CaptureRegex,
    SubstRegex,
    OpenFile,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChoosePurpose {
    Theme,
    JoinKind,
    Aggregator { col: usize },
}

pub enum Mode {
    Normal,
    Input {
        editor: LineEditor,
        purpose: InputPurpose,
    },
    Choose {
        title: String,
        items: Vec<String>,
        selected: usize,
        purpose: ChoosePurpose,
    },
    Menu(crate::menu::MenuState),
}

pub struct App {
    pub sheets: Vec<Sheet>,
    pub mode: Mode,
    pub theme: Theme,
    pub themes: Vec<Theme>,
    pub opts: Options,
    pub keybind_overrides: std::collections::HashMap<String, String>,
    pub clipboard: Clipboard,
    pub jobs: JobManager,
    pub rx: Receiver<UiMsg>,
    pub status_msg: Option<(Severity, String)>,
    pub prefix: String,
    pub quit: bool,
    pub dirty: bool,
    pub input_history: std::collections::HashMap<&'static str, Vec<String>>,
    /// Last search (pattern, all_cols, backward) for n/N.
    pub last_search: Option<(regex::Regex, bool, bool)>,
    /// Data rows on screen, updated each draw (page size).
    pub nscreen: u64,
    /// Screen width in cells, updated each draw.
    pub screen_width: u16,
    /// Graph payloads keyed by their placeholder sheet id.
    pub graphs: crate::canvas::GraphStore,
    /// Cell-pager payloads keyed by their placeholder sheet id.
    pub texts: crate::pager::TextStore,
}

impl App {
    pub fn new(theme: Theme, opts: Options) -> (Self, crossbeam_channel::Sender<UiMsg>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let jobs = JobManager::new(tx.clone());
        (
            App {
                sheets: Vec::new(),
                mode: Mode::Normal,
                themes: crate::theme::builtin_themes(),
                theme,
                opts,
                keybind_overrides: Default::default(),
                clipboard: Clipboard::default(),
                jobs,
                rx,
                status_msg: None,
                prefix: String::new(),
                quit: false,
                dirty: true,
                input_history: Default::default(),
                last_search: None,
                nscreen: 25,
                screen_width: 80,
                graphs: Default::default(),
                texts: Default::default(),
            },
            tx,
        )
    }

    pub fn sheet(&self) -> Option<&Sheet> {
        self.sheets.last()
    }

    pub fn sheet_by_id(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.id == id)
    }

    pub fn push_sheet(&mut self, sheet: Sheet) {
        self.sheets.push(sheet);
        self.dirty = true;
    }

    pub fn pop_sheet(&mut self) {
        self.sheets.pop();
        if self.sheets.is_empty() {
            self.quit = true;
        }
        self.dirty = true;
    }

    pub fn status(&mut self, sev: Severity, msg: impl Into<String>) {
        self.status_msg = Some((sev, msg.into()));
        self.dirty = true;
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.status(Severity::Error, msg);
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        self.status(Severity::Info, msg);
    }

    pub fn start_input(
        &mut self,
        purpose: InputPurpose,
        prompt: impl Into<String>,
        initial: impl Into<String>,
        completions: Vec<String>,
    ) {
        let history = self
            .input_history
            .get(history_key(purpose))
            .cloned()
            .unwrap_or_default();
        self.mode = Mode::Input {
            editor: LineEditor::new(prompt, initial)
                .with_history(history)
                .with_completions(completions),
            purpose,
        };
        self.dirty = true;
    }

    pub fn start_choose(
        &mut self,
        purpose: ChoosePurpose,
        title: impl Into<String>,
        items: Vec<String>,
        selected: usize,
    ) {
        self.mode = Mode::Choose {
            title: title.into(),
            items,
            selected,
            purpose,
        };
        self.dirty = true;
    }

    /// Set the theme by name; persist the choice to the user config.
    pub fn set_theme(&mut self, name: &str) {
        if let Some(t) = self.themes.iter().find(|t| t.name == name).cloned() {
            self.theme = t;
            if let Err(e) = crate::config::persist_theme(name) {
                self.status(Severity::Warning, format!("theme not persisted: {}", e));
            } else {
                self.info(format!("theme: {}", name));
            }
            self.dirty = true;
        } else {
            self.error(format!("unknown theme: {}", name));
        }
    }

    pub fn cycle_theme(&mut self) {
        let cur = self.theme.name;
        let idx = self.themes.iter().position(|t| t.name == cur).unwrap_or(0);
        let next = self.themes[(idx + 1) % self.themes.len()].name.to_string();
        self.set_theme(&next);
    }

    /// Drain background-job messages. Returns true if anything happened.
    pub fn drain_msgs(&mut self) -> bool {
        let mut any = false;
        while let Ok(msg) = self.rx.try_recv() {
            any = true;
            match msg {
                UiMsg::Dirty => self.dirty = true,
                UiMsg::SortDone { sheet, perm, spec } => {
                    if let Some(s) = self.sheets.iter_mut().find(|s| s.id == sheet) {
                        let old_perm = s.perm.take();
                        let old_spec = std::mem::take(&mut s.sort_spec);
                        // keep the cursor on the same row across the re-order
                        let cur_rid = s.rowid_at(s.view.cur_row);
                        s.perm = Some(perm.clone());
                        s.sort_spec = spec.clone();
                        if let Some(rid) = cur_rid {
                            if let Some(vi) = perm.iter().position(|&r| r == rid) {
                                s.view.cur_row = vi as u64;
                            }
                        }
                        s.undo.push(
                            "sort",
                            vec![rustidata_core::undo::UndoOp::SetPerm {
                                old: old_perm,
                                new: Some(perm),
                                old_spec,
                                new_spec: spec,
                            }],
                        );
                        self.dirty = true;
                    }
                }
                UiMsg::SheetReady(sheet) => {
                    self.push_sheet(*sheet);
                }
                UiMsg::Status(sev, msg) => self.status(sev, msg),
                UiMsg::JobFinished { name } => {
                    self.jobs.reap(&name);
                    self.dirty = true;
                }
            }
        }
        any
    }

    /// Record an input value into per-purpose history.
    pub fn record_history(&mut self, purpose: InputPurpose, value: &str) {
        if value.is_empty() {
            return;
        }
        let h = self.input_history.entry(history_key(purpose)).or_default();
        if h.last().map(|s| s.as_str()) != Some(value) {
            h.push(value.to_string());
        }
        if h.len() > 100 {
            h.remove(0);
        }
    }

    // ---- cell mutation with undo ----

    /// Set the cursor cell from an input string (typed through the column).
    pub fn set_cell_text(&mut self, text: &str) {
        let Some(sheet) = self.sheets.last_mut() else { return };
        let Some(rid) = sheet.cursor_rowid() else { return };
        let Some(ci) = sheet.cursor_col_index() else { return };
        let new = if text.is_empty() {
            Value::Null
        } else {
            Value::Str(text.into())
        };
        match set_column_cell(sheet, ci, rid, new) {
            Ok(()) => self.dirty = true,
            Err(e) => self.error(e),
        }
    }
}

fn history_key(purpose: InputPurpose) -> &'static str {
    use InputPurpose::*;
    match purpose {
        EditCell | EditPagerCell | EditCellsSelected => "edit",
        Search { .. } => "search",
        SelectRegex { .. } => "select",
        SelectExpr { .. } | AddExprColumn => "expr",
        RenameColumn | RenameSheet => "rename",
        SaveAs | OpenFile => "path",
        ResizeColumn | GotoRow => "number",
        ExecLongname => "longname",
        SplitRegex | CaptureRegex | SubstRegex => "regex",
    }
}

/// Set a cell on a sheet column (Source or Frozen), pushing an undo entry.
pub fn set_column_cell(
    sheet: &mut Sheet,
    ci: usize,
    rid: rustidata_core::store::RowId,
    new: Value,
) -> std::result::Result<(), String> {
    use rustidata_core::column::ColumnKind;
    match &sheet.columns[ci].kind {
        ColumnKind::Source(idx) => {
            let idx = *idx;
            let old = sheet.overlay.set_cell(rid, idx, new.clone());
            sheet.undo.push(
                "edit cell",
                vec![rustidata_core::undo::UndoOp::SetCell {
                    row: rid,
                    col: idx,
                    old,
                    new,
                }],
            );
            Ok(())
        }
        ColumnKind::Frozen(map) => {
            map.lock().insert(rid, new);
            Ok(())
        }
        _ => Err("cannot edit a computed column (freeze it first with ')".into()),
    }
}

/// Run the TUI until quit.
pub fn run(app: &mut App) -> Result<()> {
    let mut stdout = std::io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
    let result = mainloop(app, &mut stdout);
    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}

fn mainloop(app: &mut App, out: &mut impl Write) -> Result<()> {
    let (mut w, mut h) = terminal::size()?;
    if w == 0 || h == 0 {
        (w, h) = (80, 24); // degenerate pty (CI, script(1)); resize events correct this
    }
    let mut sb = ScreenBuffer::new(w, h, app.theme.get(StyleKey::Default));

    while !app.quit {
        app.drain_msgs();

        if app.dirty {
            draw_all(app, &mut sb);
            sb.flush(out)?;
            app.dirty = false;
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(kev) if kev.kind != KeyEventKind::Release => {
                    app.status_msg = None;
                    if let Some(ks) = crate::keys::keystroke(&kev) {
                        handle_key(app, &ks);
                    }
                    app.dirty = true;
                }
                Event::Resize(nw, nh) => {
                    sb.resize(nw, nh);
                    app.dirty = true;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw_all(app: &mut App, sb: &mut ScreenBuffer) {
    sb.set_default_style(app.theme.get(StyleKey::Default));
    sb.clear();
    let h = sb.height();
    if h < 4 {
        return;
    }
    // layout: menu bar (0), table (1..h-1), status (h-1)
    let table_y = 1u16;
    let table_h = h - 2;
    let nscreen = (table_h - 1) as u64;
    app.nscreen = nscreen;

    let opts = app.opts.clone();
    let theme = app.theme.clone();
    let width = sb.width();
    app.screen_width = width;

    if let Some(sheet) = app.sheets.last_mut() {
        sheet.clamp_view(nscreen);
        crate::draw_sheet::ensure_cursor_col_visible(sheet, &opts, width, nscreen);
    }
    if let Some(sheet) = app.sheets.last() {
        if sheet.kind == rustidata_core::sheet::SheetKind::Graph {
            if let Some(g) = app.graphs.get(&sheet.id) {
                crate::canvas::draw_graph(sb, &theme, g, sheet.id, table_y, table_h);
            }
        } else if sheet.kind == rustidata_core::sheet::SheetKind::Text {
            if let Some(tv) = app.texts.get_mut(&sheet.id) {
                crate::pager::draw_text_pager(sb, &theme, tv, table_y, table_h);
            }
        } else {
            crate::draw_sheet::draw(sb, sheet, &opts, &theme, table_y, table_h);
        }
        crate::statusbar::draw_status(
            sb,
            sheet,
            &theme,
            app.status_msg.as_ref(),
            &app.prefix,
            h - 1,
        );
    }

    crate::menu::draw_menu_bar(sb, app, 0);

    match &app.mode {
        Mode::Input { editor, .. } => {
            // input line replaces the status bar
            let style = theme
                .get(StyleKey::Default)
                .merge_over(&theme.get(StyleKey::EditCell));
            let line = format!("{}{}", editor.prompt, editor.buffer);
            let text = crate::cliptext::fit(&line, width as usize, "…");
            sb.put_str(0, h - 1, &text, style);
            // visible cursor block
            let cx = crate::cliptext::dispwidth(&editor.prompt)
                + crate::cliptext::dispwidth(
                    &editor.buffer.chars().take(editor.cursor).collect::<String>(),
                );
            if (cx as u16) < width {
                sb.style_region(
                    cx as u16,
                    h - 1,
                    1,
                    crate::theme::parse_style("reverse"),
                );
            }
        }
        Mode::Choose {
            title,
            items,
            selected,
            ..
        } => {
            crate::menu::draw_chooser(sb, &theme, title, items, *selected);
        }
        Mode::Menu(state) => {
            crate::menu::draw_open_menu(sb, app, state);
        }
        Mode::Normal => {}
    }
}

fn handle_key(app: &mut App, ks: &str) {
    match &mut app.mode {
        Mode::Normal => handle_normal_key(app, ks),
        Mode::Input { .. } => handle_input_key(app, ks),
        Mode::Choose { .. } => handle_choose_key(app, ks),
        Mode::Menu(_) => crate::menu::handle_menu_key(app, ks),
    }
}

fn handle_normal_key(app: &mut App, ks: &str) {
    // graph / pager sheets capture their navigation keys first
    if app.prefix.is_empty() && crate::canvas::handle_graph_key(app, ks) {
        return;
    }
    if app.prefix.is_empty() && crate::pager::handle_text_key(app, ks) {
        return;
    }
    // prefix accumulation: only g, z, gz are prefixes — anything else
    // (e.g. "gg") dispatches as a complete chord
    if ks == "g" || ks == "z" {
        let candidate = format!("{}{}", app.prefix, ks);
        if matches!(candidate.as_str(), "g" | "z" | "gz") {
            app.prefix = candidate;
            return;
        }
    }
    let full = format!("{}{}", app.prefix, ks);
    app.prefix.clear();

    if ks == "Esc" {
        return;
    }

    let longname = app
        .keybind_overrides
        .get(&full)
        .cloned()
        .or_else(|| crate::commands::lookup(&full).map(|s| s.to_string()));

    match longname {
        Some(name) => {
            if let Err(e) = crate::commands::execute(app, &name) {
                app.error(format!("{}: {}", name, e));
            }
        }
        None => {
            app.status(Severity::Warning, format!("no command bound to '{}'", full));
        }
    }
}

fn handle_input_key(app: &mut App, ks: &str) {
    let Mode::Input { editor, purpose } = &mut app.mode else {
        return;
    };
    let purpose = *purpose;
    match ks {
        "Esc" | "^C" => {
            app.mode = Mode::Normal;
        }
        "Enter" => {
            let text = editor.buffer.clone();
            app.mode = Mode::Normal;
            app.record_history(purpose, &text);
            if let Err(e) = crate::commands::finish_input(app, purpose, &text) {
                app.error(e.to_string());
            }
        }
        "Backspace" => editor.backspace(),
        "Delete" => editor.delete(),
        "Left" => editor.left(),
        "Right" => editor.right(),
        "Home" | "^A" => editor.home(),
        "End" | "^E" => editor.end(),
        "^K" => editor.clear_to_end(),
        "^U" => editor.clear_all(),
        "Up" => editor.history_prev(),
        "Down" => editor.history_next(),
        "Tab" => editor.complete(),
        s => {
            let mut chars = s.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                editor.insert(c);
            }
        }
    }
}

fn handle_choose_key(app: &mut App, ks: &str) {
    let Mode::Choose {
        items,
        selected,
        purpose,
        ..
    } = &mut app.mode
    else {
        return;
    };
    match ks {
        "Esc" | "q" | "^C" => {
            app.mode = Mode::Normal;
        }
        "j" | "Down" => {
            *selected = (*selected + 1).min(items.len().saturating_sub(1));
            // live preview for themes
            if let ChoosePurpose::Theme = purpose {
                let name = items[*selected].clone();
                if let Some(t) = app.themes.iter().find(|t| t.name == name).cloned() {
                    app.theme = t;
                }
            }
        }
        "k" | "Up" => {
            *selected = selected.saturating_sub(1);
            if let ChoosePurpose::Theme = purpose {
                let name = items[*selected].clone();
                if let Some(t) = app.themes.iter().find(|t| t.name == name).cloned() {
                    app.theme = t;
                }
            }
        }
        "Enter" => {
            let choice = items.get(*selected).cloned().unwrap_or_default();
            let purpose = *purpose;
            app.mode = Mode::Normal;
            if let Err(e) = crate::commands::finish_choose(app, purpose, &choice) {
                app.error(e.to_string());
            }
        }
        _ => {}
    }
}

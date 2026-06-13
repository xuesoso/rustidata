//! Command registry: visidata-compatible longnames and default keybindings,
//! the dispatcher, and input/chooser completion handlers.

use anyhow::{anyhow, bail, Result};
use rustidata_core::aggregate::Aggregator;
use rustidata_core::jobs::{CancelToken, Severity, UiMsg};
use rustidata_core::sheet::{Sheet, SheetKind};
use rustidata_core::store::RowId;
use rustidata_core::transform;
use rustidata_core::undo::UndoOp;
use rustidata_core::value::{VType, Value};

use crate::app::{App, ChoosePurpose, InputPurpose, Mode};

/// (keystroke, longname, help) — keystroke is the full prefixed form.
pub static BINDINGS: &[(&str, &str, &str)] = &[
    // navigation
    ("j", "go-down", "move cursor down one row"),
    ("Down", "go-down", "move cursor down one row"),
    ("k", "go-up", "move cursor up one row"),
    ("Up", "go-up", "move cursor up one row"),
    ("h", "go-left", "move cursor left one column"),
    ("Left", "go-left", "move cursor left one column"),
    ("l", "go-right", "move cursor right one column"),
    ("Right", "go-right", "move cursor right one column"),
    ("gj", "go-bottom", "move cursor to last row"),
    ("G", "go-bottom", "move cursor to last row"),
    ("End", "go-bottom", "move cursor to last row"),
    ("gk", "go-top", "move cursor to first row"),
    ("gg", "go-top", "move cursor to first row"),
    ("Home", "go-top", "move cursor to first row"),
    ("gh", "go-leftmost", "move cursor to first column"),
    ("gl", "go-rightmost", "move cursor to last column"),
    ("PgDn", "next-page", "scroll down one page"),
    ("^F", "next-page", "scroll down one page"),
    ("PgUp", "prev-page", "scroll up one page"),
    ("^B", "prev-page", "scroll up one page"),
    ("zr", "go-row-input", "go to row number"),
    // column ops
    ("-", "hide-col", "hide current column"),
    ("_", "resize-col-max", "toggle column width between longest visible value and default"),
    ("z_", "resize-col-input", "set current column width"),
    (">", "resize-col-wider", "increase current column width by 1"),
    ("<", "resize-col-narrower", "decrease current column width by 1"),
    ("!", "key-col", "toggle current column as key column"),
    ("^", "rename-col", "rename current column"),
    ("H", "slide-left", "move current column left"),
    ("L", "slide-right", "move current column right"),
    ("~", "type-string", "set column type to string"),
    ("#", "type-int", "set column type to int"),
    ("%", "type-float", "set column type to float"),
    ("@", "type-date", "set column type to date"),
    ("$", "type-currency", "set column type to currency"),
    ("z#", "type-len", "set column type to length"),
    ("z~", "type-any", "set column type to any"),
    // sort
    ("[", "sort-asc", "sort ascending by current column"),
    ("]", "sort-desc", "sort descending by current column"),
    ("z[", "sort-asc-add", "add current column to sort (ascending)"),
    ("z]", "sort-desc-add", "add current column to sort (descending)"),
    // search
    ("/", "search-col", "search current column forward by regex"),
    ("?", "searchr-col", "search current column backward by regex"),
    ("g/", "search-all", "search all columns forward by regex"),
    ("g?", "searchr-all", "search all columns backward by regex"),
    ("n", "search-next", "repeat last search forward"),
    ("N", "search-prev", "repeat last search backward"),
    // selection
    ("s", "select-row", "select current row"),
    ("t", "toggle-row", "toggle selection of current row"),
    ("u", "unselect-row", "unselect current row"),
    ("gs", "select-all", "select all rows"),
    ("gt", "toggle-all", "toggle all selections"),
    ("gu", "unselect-all", "unselect all rows"),
    ("|", "select-col-regex", "select rows matching regex in current column"),
    ("\\", "unselect-col-regex", "unselect rows matching regex in current column"),
    ("g|", "select-regex", "select rows matching regex in any column"),
    ("g\\", "unselect-regex", "unselect rows matching regex in any column"),
    ("z|", "select-expr", "select rows where expression is true"),
    ("z\\", "unselect-expr", "unselect rows where expression is true"),
    // editing
    ("e", "edit-cell", "edit current cell"),
    ("ge", "edit-cells", "set current column for selected rows"),
    ("a", "add-row", "append a blank row"),
    ("d", "delete-row", "delete current row"),
    ("gd", "delete-selected", "delete selected rows"),
    ("zd", "clear-cell", "set current cell to null"),
    ("U", "undo-last", "undo last change"),
    ("R", "redo-last", "redo last undone change"),
    // clipboard
    ("y", "copy-row", "yank current row"),
    ("gy", "copy-selected", "yank selected rows"),
    ("zy", "copy-cell", "yank current cell"),
    ("p", "paste-row", "paste yanked rows after cursor"),
    ("zp", "paste-cell", "paste yanked cell"),
    // sheets
    ("q", "quit-sheet", "close current sheet"),
    ("gq", "quit-all", "quit rustidata"),
    ("S", "sheets-sheet", "open sheets sheet"),
    ("C", "columns-sheet", "open columns sheet"),
    ("O", "options-sheet", "open options sheet"),
    ("z?", "help-commands", "open command help"),
    ("F1", "help-commands", "open command help"),
    ("Enter", "open-row", "dive into current row"),
    ("^R", "reload-sheet", "reload sheet from source"),
    ("o", "open-file", "open a file"),
    ("^S", "save-sheet", "save sheet to file"),
    ("\"", "dup-selected", "duplicate sheet with selected rows"),
    ("g\"", "dup-sheet", "duplicate entire sheet"),
    // derived sheets
    ("F", "open-freq", "frequency table of current column"),
    ("gF", "open-freq-keys", "frequency table of key columns"),
    ("W", "open-pivot", "pivot on current column (group by key columns)"),
    ("M", "open-melt", "melt non-key columns into variable/value rows"),
    ("T", "open-transpose", "transpose rows and columns"),
    ("zM", "open-unfurl", "unfurl list values in current column"),
    ("I", "open-describe", "summary statistics of all columns"),
    ("&", "open-join", "join the top two sheets on key columns"),
    ("f", "setcol-fill", "fill nulls in current column with previous value"),
    ("'", "freeze-col", "freeze current column values"),
    ("g'", "freeze-sheet", "freeze whole sheet into static copy"),
    // expressions / regex columns
    ("=", "addcol-expr", "add column computed from expression"),
    ("g=", "setcol-expr", "set current column for selected rows by expression"),
    (":", "addcol-split", "add columns by splitting current column on regex"),
    (";", "addcol-capture", "add columns from regex capture groups"),
    ("*", "addcol-subst", "add column with regex substitution"),
    // plotting
    (".", "plot-column", "scatter plot of current column vs key column"),
    // themes (rustidata addition)
    ("F8", "theme-choose", "choose color theme"),
    ("F9", "theme-cycle", "cycle to next color theme"),
    // misc
    (" ", "exec-longname", "execute command by name"),
    ("^C", "cancel-jobs", "cancel background jobs"),
    ("F10", "menu-open", "open the menu"),
    ("Alt+m", "menu-open", "open the menu"),
];

pub fn lookup(keystroke: &str) -> Option<&'static str> {
    BINDINGS
        .iter()
        .find(|(ks, _, _)| *ks == keystroke)
        .map(|(_, ln, _)| *ln)
}

pub fn all_longnames() -> Vec<String> {
    let mut names: Vec<String> = BINDINGS.iter().map(|(_, ln, _)| ln.to_string()).collect();
    names.sort();
    names.dedup();
    names
}

macro_rules! sheet_or_return {
    ($app:expr) => {
        match $app.sheets.last_mut() {
            Some(s) => s,
            None => return Ok(()),
        }
    };
}

pub fn execute(app: &mut App, longname: &str) -> Result<()> {
    match longname {
        // ---- navigation ----
        "go-down" => sheet_or_return!(app).move_cursor_rows(1),
        "go-up" => sheet_or_return!(app).move_cursor_rows(-1),
        "go-left" => sheet_or_return!(app).move_cursor_cols(-1),
        "go-right" => sheet_or_return!(app).move_cursor_cols(1),
        "go-top" => sheet_or_return!(app).goto_first_row(),
        "go-bottom" => sheet_or_return!(app).goto_last_row(),
        "go-leftmost" => sheet_or_return!(app).goto_first_col(),
        "go-rightmost" => sheet_or_return!(app).goto_last_col(),
        "next-page" => {
            let n = app.nscreen as i64;
            let s = sheet_or_return!(app);
            s.move_cursor_rows(n);
            s.view.top_row = (s.view.top_row + n as u64).min(s.nrows().saturating_sub(1));
        }
        "prev-page" => {
            let n = app.nscreen as i64;
            let s = sheet_or_return!(app);
            s.move_cursor_rows(-n);
            s.view.top_row = s.view.top_row.saturating_sub(n as u64);
        }
        "go-row-input" => {
            app.start_input(InputPurpose::GotoRow, "go to row: ", "", vec![]);
        }

        // ---- column ops ----
        "hide-col" => with_columns_undo(app, "hide column", |s| {
            if let Some(ci) = s.cursor_col_index() {
                s.columns[ci].hidden = true;
            }
        })?,
        "resize-col-max" => {
            let nscreen = app.nscreen;
            let cap = app.screen_width;
            with_columns_undo(app, "resize column", |s| {
                if let Some(ci) = s.cursor_col_index() {
                    let maxw = crate::draw_sheet::max_col_width(s, ci, nscreen, cap);
                    let col = &mut s.columns[ci];
                    col.width = if col.width == Some(maxw) {
                        None // collapse back to default auto-fit
                    } else {
                        Some(maxw)
                    };
                }
            })?
        }
        "resize-col-wider" | "resize-col-narrower" => {
            let nscreen = app.nscreen;
            let opts = app.opts.clone();
            let delta: i32 = if longname == "resize-col-wider" { 1 } else { -1 };
            with_columns_undo(app, "resize column", |s| {
                if let Some(ci) = s.cursor_col_index() {
                    let cur = crate::draw_sheet::col_width(s, &opts, ci, nscreen) as i32;
                    let w = (cur + delta).max(crate::draw_sheet::MIN_COL_WIDTH as i32);
                    s.columns[ci].width = Some(w as u16);
                }
            })?
        }
        "resize-col-input" => {
            let cur = app
                .sheet()
                .and_then(|s| s.cursor_column())
                .and_then(|c| c.width)
                .map(|w| w.to_string())
                .unwrap_or_default();
            app.start_input(InputPurpose::ResizeColumn, "width: ", cur, vec![]);
        }
        "key-col" => with_columns_undo(app, "toggle key column", |s| {
            if let Some(ci) = s.cursor_col_index() {
                if s.columns[ci].key_pos.is_some() {
                    s.columns[ci].key_pos = None;
                } else {
                    let next = s
                        .columns
                        .iter()
                        .filter_map(|c| c.key_pos)
                        .max()
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    s.columns[ci].key_pos = Some(next);
                }
            }
        })?,
        "rename-col" => {
            let cur = app
                .sheet()
                .and_then(|s| s.cursor_column())
                .map(|c| c.name.clone())
                .unwrap_or_default();
            app.start_input(InputPurpose::RenameColumn, "column name: ", cur, vec![]);
        }
        "slide-left" => with_columns_undo(app, "move column", |s| {
            if let Some(ci) = s.cursor_col_index() {
                if ci > 0 {
                    s.columns.swap(ci, ci - 1);
                    s.move_cursor_cols(-1);
                }
            }
        })?,
        "slide-right" => with_columns_undo(app, "move column", |s| {
            if let Some(ci) = s.cursor_col_index() {
                if ci + 1 < s.columns.len() {
                    s.columns.swap(ci, ci + 1);
                    s.move_cursor_cols(1);
                }
            }
        })?,
        "type-string" => set_type(app, VType::Str)?,
        "type-int" => set_type(app, VType::Int)?,
        "type-float" => set_type(app, VType::Float)?,
        "type-date" => set_type(app, VType::Date)?,
        "type-currency" => set_type(app, VType::Currency)?,
        "type-len" => set_type(app, VType::Len)?,
        "type-any" => set_type(app, VType::Any)?,

        // ---- sort ----
        "sort-asc" => start_sort(app, false, false),
        "sort-desc" => start_sort(app, true, false),
        "sort-asc-add" => start_sort(app, false, true),
        "sort-desc-add" => start_sort(app, true, true),

        // ---- search ----
        "search-col" => app.start_input(
            InputPurpose::Search {
                backward: false,
                all_cols: false,
            },
            "/",
            "",
            vec![],
        ),
        "searchr-col" => app.start_input(
            InputPurpose::Search {
                backward: true,
                all_cols: false,
            },
            "?",
            "",
            vec![],
        ),
        "search-all" => app.start_input(
            InputPurpose::Search {
                backward: false,
                all_cols: true,
            },
            "g/",
            "",
            vec![],
        ),
        "searchr-all" => app.start_input(
            InputPurpose::Search {
                backward: true,
                all_cols: true,
            },
            "g?",
            "",
            vec![],
        ),
        "search-next" => repeat_search(app, false)?,
        "search-prev" => repeat_search(app, true)?,

        // ---- selection ----
        "select-row" => {
            let s = sheet_or_return!(app);
            if let Some(rid) = s.cursor_rowid() {
                s.selection.insert(rid);
                s.move_cursor_rows(1);
            }
        }
        "toggle-row" => {
            let s = sheet_or_return!(app);
            s.toggle_select_cursor();
            s.move_cursor_rows(1);
        }
        "unselect-row" => {
            let s = sheet_or_return!(app);
            if let Some(rid) = s.cursor_rowid() {
                s.selection.remove(rid);
                s.move_cursor_rows(1);
            }
        }
        "select-all" => with_selection_undo(app, "select all", |s| {
            for rid in s.iter_rowids().collect::<Vec<_>>() {
                s.selection.insert(rid);
            }
        })?,
        "unselect-all" => with_selection_undo(app, "unselect all", |s| {
            s.selection.clear();
        })?,
        "toggle-all" => with_selection_undo(app, "toggle all", |s| {
            for rid in s.iter_rowids().collect::<Vec<_>>() {
                if s.selection.contains(rid) {
                    s.selection.remove(rid);
                } else {
                    s.selection.insert(rid);
                }
            }
        })?,
        "select-col-regex" => app.start_input(
            InputPurpose::SelectRegex {
                unselect: false,
                all_cols: false,
            },
            "select regex: ",
            "",
            vec![],
        ),
        "unselect-col-regex" => app.start_input(
            InputPurpose::SelectRegex {
                unselect: true,
                all_cols: false,
            },
            "unselect regex: ",
            "",
            vec![],
        ),
        "select-regex" => app.start_input(
            InputPurpose::SelectRegex {
                unselect: false,
                all_cols: true,
            },
            "select regex (all cols): ",
            "",
            vec![],
        ),
        "unselect-regex" => app.start_input(
            InputPurpose::SelectRegex {
                unselect: true,
                all_cols: true,
            },
            "unselect regex (all cols): ",
            "",
            vec![],
        ),
        "select-expr" => app.start_input(
            InputPurpose::SelectExpr { unselect: false },
            "select expr: ",
            "",
            vec![],
        ),
        "unselect-expr" => app.start_input(
            InputPurpose::SelectExpr { unselect: true },
            "unselect expr: ",
            "",
            vec![],
        ),

        // ---- editing ----
        "edit-cell" => {
            let initial = app
                .sheet()
                .and_then(|s| {
                    let rid = s.cursor_rowid()?;
                    let ci = s.cursor_col_index()?;
                    Some(s.columns[ci].display_value(s, rid).to_string())
                })
                .unwrap_or_default();
            app.start_input(InputPurpose::EditCell, "edit: ", initial, vec![]);
        }
        "edit-cells" => {
            app.start_input(InputPurpose::EditCellsSelected, "set selected to: ", "", vec![]);
        }
        "clear-cell" => app.set_cell_text(""),
        "add-row" => {
            let s = sheet_or_return!(app);
            let ncols = s.source.ncols();
            let rid = s.overlay.insert_row(vec![Value::Null; ncols]);
            if let Some(p) = &mut s.perm {
                let v = std::sync::Arc::make_mut(p);
                let at = (s.view.cur_row as usize + 1).min(v.len());
                v.insert(at, rid);
            }
            s.undo.push("add row", vec![UndoOp::DeleteRows { rows: vec![rid] }]);
            app.info("row added at end (natural order)");
        }
        "delete-row" => {
            let s = sheet_or_return!(app);
            if let Some(rid) = s.cursor_rowid() {
                delete_rows(s, vec![rid]);
            }
        }
        "delete-selected" => {
            let s = sheet_or_return!(app);
            let rows: Vec<RowId> = s.selection.iter().collect();
            if rows.is_empty() {
                app.error("no rows selected");
            } else {
                let s = sheet_or_return!(app);
                delete_rows(s, rows);
            }
        }
        "undo-last" => {
            let s = sheet_or_return!(app);
            match s.undo.pop_undo() {
                Some(entry) => {
                    apply_ops(s, &entry.ops, true);
                    let label = entry.label.clone();
                    s.undo.push_undone(entry);
                    app.info(format!("undid {}", label));
                }
                None => app.error("nothing to undo"),
            }
        }
        "redo-last" => {
            let s = sheet_or_return!(app);
            match s.undo.pop_redo() {
                Some(entry) => {
                    apply_ops(s, &entry.ops, false);
                    let label = entry.label.clone();
                    s.undo.push_redone(entry);
                    app.info(format!("redid {}", label));
                }
                None => app.error("nothing to redo"),
            }
        }

        // ---- clipboard ----
        "copy-row" => {
            let s = sheet_or_return!(app);
            if let Some(rid) = s.cursor_rowid() {
                let row = realize_row(s, rid);
                app.clipboard.copy_rows(vec![row]);
                app.info("1 row yanked");
            }
        }
        "copy-selected" => {
            let s = sheet_or_return!(app);
            let rows: Vec<Vec<Value>> = s
                .selected_or_all()
                .iter()
                .map(|&rid| realize_row(s, rid))
                .collect();
            let n = rows.len();
            app.clipboard.copy_rows(rows);
            app.info(format!("{} rows yanked", n));
        }
        "copy-cell" => {
            let s = sheet_or_return!(app);
            if let (Some(rid), Some(ci)) = (s.cursor_rowid(), s.cursor_col_index()) {
                let v = s.columns[ci].typed_value(s, rid);
                app.clipboard.copy_cell(v);
                app.info("cell yanked");
            }
        }
        "paste-row" => {
            let rows = app.clipboard.rows.clone();
            if rows.is_empty() {
                bail!("clipboard is empty");
            }
            let s = sheet_or_return!(app);
            let vis = s.visible_cols();
            let ncols = s.source.ncols();
            let mut new_ids = Vec::new();
            for row in rows {
                let mut values = vec![Value::Null; ncols];
                for (vi, v) in row.into_iter().enumerate() {
                    if let Some(&ci) = vis.get(vi) {
                        if let rustidata_core::column::ColumnKind::Source(si) =
                            s.columns[ci].kind
                        {
                            if si < ncols {
                                values[si] = v;
                            }
                        }
                    }
                }
                let rid = s.overlay.insert_row(values);
                if let Some(p) = &mut s.perm {
                    let v = std::sync::Arc::make_mut(p);
                    let at = (s.view.cur_row as usize + 1).min(v.len());
                    v.insert(at, rid);
                }
                new_ids.push(rid);
            }
            s.undo.push("paste rows", vec![UndoOp::DeleteRows { rows: new_ids }]);
        }
        "paste-cell" => {
            let v = app.clipboard.cell.clone();
            match v {
                Some(v) => {
                    let s = sheet_or_return!(app);
                    if let (Some(rid), Some(ci)) = (s.cursor_rowid(), s.cursor_col_index()) {
                        crate::app::set_column_cell(s, ci, rid, v).map_err(|e| anyhow!(e))?;
                    }
                }
                None => bail!("no cell yanked"),
            }
        }

        // ---- sheet stack ----
        "quit-sheet" => app.pop_sheet(),
        "quit-all" => app.quit = true,
        "sheets-sheet" => {
            let s = crate::metasheets::sheets_sheet(app);
            app.push_sheet(s);
        }
        "columns-sheet" => {
            if let Some(sheet) = app.sheet() {
                let s = crate::metasheets::columns_sheet(sheet);
                app.push_sheet(s);
            }
        }
        "options-sheet" => {
            let s = crate::metasheets::options_sheet(app);
            app.push_sheet(s);
        }
        "help-commands" => {
            let s = crate::metasheets::help_sheet();
            app.push_sheet(s);
        }
        "open-row" => open_row(app)?,
        "reload-sheet" => reload_sheet(app)?,
        "open-file" => app.start_input(InputPurpose::OpenFile, "open: ", "", vec![]),
        "save-sheet" => {
            let initial = app
                .sheet()
                .and_then(|s| s.source_path.as_ref())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            app.start_input(InputPurpose::SaveAs, "save to: ", initial, vec![]);
        }
        "rename-sheet" => {
            let cur = app.sheet().map(|s| s.name.clone()).unwrap_or_default();
            app.start_input(InputPurpose::RenameSheet, "sheet name: ", cur, vec![]);
        }
        "dup-sheet" => {
            let s = sheet_or_return!(app);
            let dup = transform::subset_sheet(
                s,
                format!("{}_copy", s.name),
                s.iter_rowids().collect(),
            );
            app.push_sheet(dup);
        }
        "dup-selected" => {
            let s = sheet_or_return!(app);
            let ids = s.selected_or_all();
            let dup = transform::subset_sheet(s, format!("{}_copy", s.name), ids);
            app.push_sheet(dup);
        }

        // ---- derived sheets ----
        "open-freq" => {
            let s = sheet_or_return!(app);
            let Some(ci) = s.cursor_col_index() else { return Ok(()) };
            spawn_transform(app, "freq", move |snap, cancel| {
                transform::freq::freq_table(snap, &[ci], &[], cancel)
            });
        }
        "open-freq-keys" => {
            let s = sheet_or_return!(app);
            let keys: Vec<usize> = (0..s.columns.len())
                .filter(|&i| s.columns[i].is_key())
                .collect();
            if keys.is_empty() {
                bail!("no key columns (set with !)");
            }
            spawn_transform(app, "freq", move |snap, cancel| {
                transform::freq::freq_table(snap, &keys, &[], cancel)
            });
        }
        "open-pivot" => {
            let s = sheet_or_return!(app);
            let has_keys = s.columns.iter().any(|c| c.is_key());
            if !has_keys {
                bail!("set key column(s) with ! first (group-by columns)");
            }
            if s.cursor_col_index().is_none() {
                return Ok(());
            }
            let aggs: Vec<String> = Aggregator::all().iter().map(|a| a.name().to_string()).collect();
            let col = s.cursor_col_index().unwrap();
            app.start_choose(
                ChoosePurpose::Aggregator { col },
                "pivot aggregator",
                aggs,
                3, // sum
            );
        }
        "open-melt" => spawn_transform(app, "melt", move |snap, cancel| {
            transform::melt::melt(snap, cancel)
        }),
        "open-transpose" => spawn_transform(app, "transpose", move |snap, cancel| {
            transform::transpose::transpose(snap, cancel)
        }),
        "open-unfurl" => {
            let s = sheet_or_return!(app);
            let Some(ci) = s.cursor_col_index() else { return Ok(()) };
            spawn_transform(app, "unfurl", move |snap, cancel| {
                transform::unfurl::unfurl(snap, ci, cancel)
            });
        }
        "open-describe" => spawn_transform(app, "describe", move |snap, cancel| {
            transform::describe::describe(snap, cancel)
        }),
        "open-join" => {
            if app.sheets.len() < 2 {
                bail!("need at least 2 sheets on the stack to join");
            }
            let kinds: Vec<String> = transform::join::JoinKind::all_names()
                .iter()
                .map(|s| s.to_string())
                .collect();
            app.start_choose(ChoosePurpose::JoinKind, "join type", kinds, 0);
        }
        "setcol-fill" => {
            let s = sheet_or_return!(app);
            if let Some(ci) = s.cursor_col_index() {
                let ops = transform::fill::fill_forward(s, ci);
                let n = ops.len();
                if !ops.is_empty() {
                    s.undo.push("fill", ops);
                }
                app.info(format!("filled {} cells", n));
            }
        }
        "freeze-col" => {
            let s = sheet_or_return!(app);
            if let Some(ci) = s.cursor_col_index() {
                transform::freeze::freeze_column(s, ci);
                app.info("column frozen");
            }
        }
        "freeze-sheet" => {
            let s = sheet_or_return!(app);
            let frozen = transform::freeze::freeze_sheet(s);
            app.push_sheet(frozen);
        }

        // ---- expressions / regex columns ----
        "addcol-expr" => app.start_input(
            InputPurpose::AddExprColumn,
            "new column expr (name=expr): ",
            "",
            vec![],
        ),
        "setcol-expr" => app.start_input(
            InputPurpose::EditCellsSelected,
            "set selected to expr: =",
            "",
            vec![],
        ),
        "addcol-split" => app.start_input(InputPurpose::SplitRegex, "split regex: ", "", vec![]),
        "addcol-capture" => {
            app.start_input(InputPurpose::CaptureRegex, "capture regex: ", "", vec![])
        }
        "addcol-subst" => app.start_input(
            InputPurpose::SubstRegex,
            "subst (regex/replacement): ",
            "",
            vec![],
        ),

        // ---- plotting ----
        "plot-column" => crate::canvas::open_graph(app)?,

        // ---- themes ----
        "theme-choose" => {
            let names: Vec<String> = app.themes.iter().map(|t| t.name.to_string()).collect();
            let cur = app
                .themes
                .iter()
                .position(|t| t.name == app.theme.name)
                .unwrap_or(0);
            app.start_choose(ChoosePurpose::Theme, "color theme", names, cur);
        }
        "theme-cycle" => app.cycle_theme(),

        // ---- misc ----
        "exec-longname" => {
            app.start_input(InputPurpose::ExecLongname, "command: ", "", all_longnames())
        }
        "cancel-jobs" => {
            app.jobs.cancel_all();
            app.info("background jobs cancelled");
        }
        "menu-open" => {
            app.mode = Mode::Menu(crate::menu::MenuState {
                menu_idx: 0,
                item_idx: 0,
            });
        }

        other => bail!("unknown command: {}", other),
    }
    Ok(())
}

// ---- helpers ----

fn realize_row(sheet: &Sheet, rid: RowId) -> Vec<Value> {
    sheet
        .visible_cols()
        .iter()
        .map(|&ci| sheet.columns[ci].typed_value(sheet, rid))
        .collect()
}

fn with_columns_undo(
    app: &mut App,
    label: &str,
    f: impl FnOnce(&mut Sheet),
) -> Result<()> {
    let Some(s) = app.sheets.last_mut() else { return Ok(()) };
    let old = s.columns.clone();
    f(s);
    s.undo.push(
        label,
        vec![UndoOp::SetColumns {
            old,
            new: s.columns.clone(),
        }],
    );
    Ok(())
}

fn with_selection_undo(
    app: &mut App,
    label: &str,
    f: impl FnOnce(&mut Sheet),
) -> Result<()> {
    let Some(s) = app.sheets.last_mut() else { return Ok(()) };
    let old = s.selection.clone();
    f(s);
    s.undo.push(
        label,
        vec![UndoOp::SetSelection {
            old,
            new: s.selection.clone(),
        }],
    );
    Ok(())
}

fn set_type(app: &mut App, vtype: VType) -> Result<()> {
    with_columns_undo(app, "set type", |s| {
        if let Some(ci) = s.cursor_col_index() {
            s.columns[ci].vtype = vtype;
            s.columns[ci].invalidate_cache();
        }
    })
}

fn delete_rows(s: &mut Sheet, rows: Vec<RowId>) {
    let mut ops = Vec::new();
    if s.perm.is_some() {
        let old = s.perm.clone();
        for &rid in &rows {
            s.remove_from_perm(rid);
        }
        ops.push(UndoOp::SetPerm {
            old,
            new: s.perm.clone(),
            old_spec: s.sort_spec.clone(),
            new_spec: s.sort_spec.clone(),
        });
    }
    for &rid in &rows {
        s.overlay.delete_row(rid);
        s.selection.remove(rid);
    }
    ops.push(UndoOp::DeleteRows { rows });
    s.undo.push("delete rows", ops);
}

pub fn apply_ops(s: &mut Sheet, ops: &[UndoOp], undo: bool) {
    // undo applies inverses in reverse order
    let iter: Box<dyn Iterator<Item = &UndoOp>> = if undo {
        Box::new(ops.iter().rev())
    } else {
        Box::new(ops.iter())
    };
    for op in iter {
        match op {
            UndoOp::SetCell { row, col, old, new } => {
                if undo {
                    match old {
                        Some(v) => {
                            s.overlay.set_cell(*row, *col, v.clone());
                        }
                        None => s.overlay.clear_cell(*row, *col),
                    }
                } else {
                    s.overlay.set_cell(*row, *col, new.clone());
                }
            }
            UndoOp::DeleteRows { rows } => {
                for &rid in rows {
                    if undo {
                        s.overlay.undelete_row(rid);
                    } else {
                        s.overlay.delete_row(rid);
                    }
                }
            }
            UndoOp::SetSelection { old, new } => {
                s.selection = if undo { old.clone() } else { new.clone() };
            }
            UndoOp::SetPerm {
                old,
                new,
                old_spec,
                new_spec,
            } => {
                if undo {
                    s.perm = old.clone();
                    s.sort_spec = old_spec.clone();
                } else {
                    s.perm = new.clone();
                    s.sort_spec = new_spec.clone();
                }
            }
            UndoOp::SetColumns { old, new } => {
                s.columns = if undo { old.clone() } else { new.clone() };
            }
        }
    }
}

fn start_sort(app: &mut App, descending: bool, additive: bool) {
    let Some(s) = app.sheets.last() else { return };
    let Some(ci) = s.cursor_col_index() else { return };
    let mut spec = if additive { s.sort_spec.clone() } else { Vec::new() };
    spec.retain(|&(c, _)| c != ci);
    spec.push((ci, descending));

    let snapshot = s.job_snapshot();
    let sid = s.id;
    let spec2 = spec.clone();
    app.jobs.spawn("sort", move |cancel, tx| {
        if let Some(perm) = rustidata_core::sort::sort_permutation(&snapshot, &spec2, &cancel) {
            let _ = tx.send(UiMsg::SortDone {
                sheet: sid,
                perm,
                spec: spec2,
            });
        }
    });
    app.info("sorting…");
}

fn spawn_transform(
    app: &mut App,
    name: &'static str,
    f: impl FnOnce(&Sheet, &CancelToken) -> Option<Sheet> + Send + 'static,
) {
    let Some(s) = app.sheets.last() else { return };
    let snapshot = s.job_snapshot();
    app.jobs.spawn(name, move |cancel, tx| {
        match f(&snapshot, &cancel) {
            Some(result) => {
                let _ = tx.send(UiMsg::SheetReady(Box::new(result)));
            }
            None => {
                let _ = tx.send(UiMsg::Status(Severity::Warning, format!("{} cancelled", name)));
            }
        }
    });
}

/// Enter: dive into the current row, depending on sheet kind.
fn open_row(app: &mut App) -> Result<()> {
    let Some(s) = app.sheet() else { return Ok(()) };
    match s.kind {
        SheetKind::Freq | SheetKind::Pivot | SheetKind::Melt | SheetKind::Unfurl => {
            let Some(rid) = s.cursor_rowid() else { return Ok(()) };
            let Some(groups) = &s.groups else {
                bail!("no source rows recorded for this sheet")
            };
            let Some(ids) = groups.get(rid as usize).cloned() else {
                bail!("no source rows for this row")
            };
            let Some(parent_id) = s.parent else {
                bail!("parent sheet is gone")
            };
            let Some(parent) = app.sheet_by_id(parent_id) else {
                bail!("parent sheet is gone")
            };
            let name = format!("{}_dive", s.name);
            let sub = transform::subset_sheet(parent, name, ids);
            app.push_sheet(sub);
        }
        SheetKind::Dir => open_dir_entry(app)?,
        SheetKind::Sheets => {
            // jump to the sheet named in the current row
            let Some(rid) = s.cursor_rowid() else { return Ok(()) };
            let id_val = s.columns.last().map(|c| c.typed_value(s, rid));
            if let Some(Value::Int(id)) = id_val {
                if let Some(pos) = app.sheets.iter().position(|sh| sh.id == id as u64) {
                    let sheet = app.sheets.remove(pos);
                    app.sheets.pop(); // drop the sheets-sheet itself
                    app.sheets.push(sheet);
                    app.dirty = true;
                }
            }
        }
        _ => app.info("nothing to dive into"),
    }
    Ok(())
}

fn open_dir_entry(app: &mut App) -> Result<()> {
    let Some(s) = app.sheet() else { return Ok(()) };
    let Some(rid) = s.cursor_rowid() else { return Ok(()) };
    let name = s.columns[0].typed_value(s, rid).to_display().to_string();
    let Some(base) = s.source_path.clone() else { return Ok(()) };

    // sqlite index sheet: open the named table
    let is_db = matches!(
        base.extension().and_then(|e| e.to_str()),
        Some("db") | Some("sqlite") | Some("sqlite3")
    );
    if is_db {
        #[cfg(feature = "sqlite")]
        {
            let table = rustidata_loaders::sqlite::load_table(&base, &name)?;
            app.push_sheet(table);
            return Ok(());
        }
        #[cfg(not(feature = "sqlite"))]
        bail!("built without sqlite support");
    }

    let path = base.join(&name);
    open_path(app, &path)
}

pub fn open_path(app: &mut App, path: &std::path::Path) -> Result<()> {
    let tx = app.jobs.sender();
    let notify = std::sync::Arc::new(move || {
        let _ = tx.send(UiMsg::Dirty);
    });
    let sheet = rustidata_loaders::open_with_context(path, &app.opts, notify)?;
    app.push_sheet(sheet);
    Ok(())
}

fn reload_sheet(app: &mut App) -> Result<()> {
    let Some(s) = app.sheet() else { return Ok(()) };
    let Some(path) = s.source_path.clone() else {
        bail!("sheet has no source file")
    };
    app.pop_sheet();
    app.quit = false; // popping the last sheet must not quit on reload
    open_path(app, &path)?;
    app.info("reloaded");
    Ok(())
}

// ---- input completion ----

pub fn finish_input(app: &mut App, purpose: InputPurpose, text: &str) -> Result<()> {
    match purpose {
        InputPurpose::EditCell => app.set_cell_text(text),
        InputPurpose::EditCellsSelected => {
            let Some(s) = app.sheets.last_mut() else { return Ok(()) };
            let Some(ci) = s.cursor_col_index() else { return Ok(()) };
            let targets = s.selected_or_all();
            // expression mode if text starts with '='
            if let Some(expr_src) = text.strip_prefix('=') {
                let ast = rustidata_core::expr::compile(expr_src).map_err(|e| anyhow!(e))?;
                let mut ops = Vec::new();
                for rid in targets {
                    let v = rustidata_core::expr::eval_row(s, rid, &ast);
                    if let rustidata_core::column::ColumnKind::Source(si) = s.columns[ci].kind {
                        let old = s.overlay.set_cell(rid, si, v.clone());
                        ops.push(UndoOp::SetCell {
                            row: rid,
                            col: si,
                            old,
                            new: v,
                        });
                    }
                }
                let n = ops.len();
                if !ops.is_empty() {
                    s.undo.push("set cells", ops);
                }
                app.info(format!("set {} cells", n));
            } else {
                let new = if text.is_empty() {
                    Value::Null
                } else {
                    Value::Str(text.into())
                };
                let mut ops = Vec::new();
                for rid in targets {
                    if let rustidata_core::column::ColumnKind::Source(si) = s.columns[ci].kind {
                        let old = s.overlay.set_cell(rid, si, new.clone());
                        ops.push(UndoOp::SetCell {
                            row: rid,
                            col: si,
                            old,
                            new: new.clone(),
                        });
                    }
                }
                let n = ops.len();
                if !ops.is_empty() {
                    s.undo.push("set cells", ops);
                }
                app.info(format!("set {} cells", n));
            }
        }
        InputPurpose::Search { backward, all_cols } => {
            let re = regex::RegexBuilder::new(text)
                .case_insensitive(text.chars().all(|c| !c.is_uppercase()))
                .build()
                .map_err(|e| anyhow!("bad regex: {}", e))?;
            app.last_search = Some((re, all_cols, backward));
            do_search(app, backward)?;
        }
        InputPurpose::SelectRegex { unselect, all_cols } => {
            let re = regex::Regex::new(text).map_err(|e| anyhow!("bad regex: {}", e))?;
            let scope = if all_cols {
                rustidata_core::search::SearchScope::AllCols
            } else {
                rustidata_core::search::SearchScope::CursorCol
            };
            with_selection_undo(app, "select by regex", |s| {
                for rid in rustidata_core::search::matching_rowids(s, &re, scope) {
                    if unselect {
                        s.selection.remove(rid);
                    } else {
                        s.selection.insert(rid);
                    }
                }
            })?;
            let n = app.sheet().map(|s| s.n_selected()).unwrap_or(0);
            app.info(format!("{} rows selected", n));
        }
        InputPurpose::SelectExpr { unselect } => {
            let ast = rustidata_core::expr::compile(text).map_err(|e| anyhow!(e))?;
            with_selection_undo(app, "select by expr", |s| {
                let ids: Vec<RowId> = s.iter_rowids().collect();
                for rid in ids {
                    if rustidata_core::expr::eval_predicate(s, rid, &ast) {
                        if unselect {
                            s.selection.remove(rid);
                        } else {
                            s.selection.insert(rid);
                        }
                    }
                }
            })?;
            let n = app.sheet().map(|s| s.n_selected()).unwrap_or(0);
            app.info(format!("{} rows selected", n));
        }
        InputPurpose::AddExprColumn => {
            let (name, expr_src) = match text.split_once('=') {
                Some((n, e)) if !n.trim().is_empty() => (n.trim().to_string(), e.trim()),
                _ => (format!("expr_{}", text.len()), text),
            };
            let ast = rustidata_core::expr::compile(expr_src).map_err(|e| anyhow!(e))?;
            with_columns_undo(app, "add expr column", |s| {
                let at = s.cursor_col_index().map(|c| c + 1).unwrap_or(s.columns.len());
                s.columns.insert(
                    at,
                    rustidata_core::column::Column::expr(name, expr_src, ast),
                );
            })?;
        }
        InputPurpose::RenameColumn => with_columns_undo(app, "rename column", |s| {
            if let Some(ci) = s.cursor_col_index() {
                s.columns[ci].name = text.to_string();
            }
        })?,
        InputPurpose::RenameSheet => {
            let Some(s) = app.sheets.last_mut() else { return Ok(()) };
            s.name = text.to_string();
        }
        InputPurpose::SaveAs => {
            let Some(s) = app.sheet() else { return Ok(()) };
            let path = std::path::PathBuf::from(text);
            let is_db = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("db") | Some("sqlite") | Some("sqlite3")
            );
            if is_db {
                #[cfg(feature = "sqlite")]
                rustidata_loaders::sqlite::save_sqlite(s, &path)?;
                #[cfg(not(feature = "sqlite"))]
                bail!("built without sqlite support");
            } else {
                rustidata_loaders::save::save(s, &path)?;
            }
            app.info(format!("saved to {}", text));
        }
        InputPurpose::ResizeColumn => {
            let w: u16 = text.parse().map_err(|_| anyhow!("not a number: {}", text))?;
            with_columns_undo(app, "resize column", |s| {
                if let Some(ci) = s.cursor_col_index() {
                    s.columns[ci].width = Some(w);
                }
            })?;
        }
        InputPurpose::ExecLongname => execute(app, text.trim())?,
        InputPurpose::GotoRow => {
            let n: u64 = text.parse().map_err(|_| anyhow!("not a number: {}", text))?;
            let Some(s) = app.sheets.last_mut() else { return Ok(()) };
            s.goto_row(n.saturating_sub(1));
        }
        InputPurpose::SplitRegex => add_regex_columns(app, text, RegexColMode::Split)?,
        InputPurpose::CaptureRegex => add_regex_columns(app, text, RegexColMode::Capture)?,
        InputPurpose::SubstRegex => add_regex_columns(app, text, RegexColMode::Subst)?,
        InputPurpose::OpenFile => {
            let path = std::path::PathBuf::from(shellexpand_home(text));
            open_path(app, &path)?;
        }
    }
    Ok(())
}

fn shellexpand_home(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    s.to_string()
}

fn do_search(app: &mut App, backward: bool) -> Result<()> {
    let Some((re, all_cols, _)) = app.last_search.clone() else {
        bail!("no search pattern")
    };
    let scope = if all_cols {
        rustidata_core::search::SearchScope::AllCols
    } else {
        rustidata_core::search::SearchScope::CursorCol
    };
    let Some(s) = app.sheets.last_mut() else { return Ok(()) };
    match rustidata_core::search::search(s, &re, scope, s.view.cur_row, backward) {
        Some(vi) => {
            s.view.cur_row = vi;
        }
        None => app.status(Severity::Warning, "no match"),
    }
    Ok(())
}

fn repeat_search(app: &mut App, invert: bool) -> Result<()> {
    let Some((_, _, backward)) = app.last_search else {
        bail!("no previous search")
    };
    do_search(app, backward ^ invert)
}

enum RegexColMode {
    Split,
    Capture,
    Subst,
}

fn add_regex_columns(app: &mut App, text: &str, mode: RegexColMode) -> Result<()> {
    let Some(s) = app.sheets.last_mut() else { return Ok(()) };
    let Some(ci) = s.cursor_col_index() else { return Ok(()) };
    let src_name = s.columns[ci].name.clone();

    match mode {
        RegexColMode::Split => {
            let re = regex::Regex::new(text).map_err(|e| anyhow!("bad regex: {}", e))?;
            // determine the number of split fields from the cursor row
            let n = s
                .cursor_rowid()
                .map(|rid| {
                    let v = s.columns[ci].display_value(s, rid);
                    re.split(&v).count()
                })
                .unwrap_or(2);
            let old = s.columns.clone();
            for k in 0..n {
                let re2 = re.clone();
                let src_col = ci;
                s.columns.insert(
                    ci + 1 + k,
                    rustidata_core::column::Column::computed(
                        format!("{}_{}", src_name, k),
                        std::sync::Arc::new(move |sheet, rid| {
                            let v = sheet.columns[src_col].display_value(sheet, rid);
                            re2.split(&v)
                                .nth(k)
                                .map(|p| {
                                    if p.is_empty() {
                                        Value::Null
                                    } else {
                                        Value::Str(p.into())
                                    }
                                })
                                .unwrap_or(Value::Null)
                        }),
                    ),
                );
            }
            s.undo.push(
                "split column",
                vec![UndoOp::SetColumns {
                    old,
                    new: s.columns.clone(),
                }],
            );
        }
        RegexColMode::Capture => {
            let re = regex::Regex::new(text).map_err(|e| anyhow!("bad regex: {}", e))?;
            let ngroups = re.captures_len() - 1;
            if ngroups == 0 {
                bail!("regex has no capture groups");
            }
            let old = s.columns.clone();
            for k in 0..ngroups {
                let re2 = re.clone();
                let src_col = ci;
                s.columns.insert(
                    ci + 1 + k,
                    rustidata_core::column::Column::computed(
                        format!("{}_re{}", src_name, k + 1),
                        std::sync::Arc::new(move |sheet, rid| {
                            let v = sheet.columns[src_col].display_value(sheet, rid);
                            re2.captures(&v)
                                .and_then(|c| c.get(k + 1))
                                .map(|m| Value::Str(m.as_str().into()))
                                .unwrap_or(Value::Null)
                        }),
                    ),
                );
            }
            s.undo.push(
                "capture column",
                vec![UndoOp::SetColumns {
                    old,
                    new: s.columns.clone(),
                }],
            );
        }
        RegexColMode::Subst => {
            let (pat, repl) = text
                .split_once('/')
                .ok_or_else(|| anyhow!("expected pattern/replacement"))?;
            let re = regex::Regex::new(pat).map_err(|e| anyhow!("bad regex: {}", e))?;
            let repl = repl.to_string();
            let old = s.columns.clone();
            let src_col = ci;
            s.columns.insert(
                ci + 1,
                rustidata_core::column::Column::computed(
                    format!("{}_sub", src_name),
                    std::sync::Arc::new(move |sheet, rid| {
                        let v = sheet.columns[src_col].display_value(sheet, rid);
                        Value::Str(re.replace_all(&v, repl.as_str()).into_owned().into())
                    }),
                ),
            );
            s.undo.push(
                "subst column",
                vec![UndoOp::SetColumns {
                    old,
                    new: s.columns.clone(),
                }],
            );
        }
    }
    Ok(())
}

// ---- chooser completion ----

pub fn finish_choose(app: &mut App, purpose: ChoosePurpose, choice: &str) -> Result<()> {
    match purpose {
        ChoosePurpose::Theme => {
            app.set_theme(&choice.to_string());
        }
        ChoosePurpose::JoinKind => {
            let kind = transform::join::JoinKind::from_name(choice)
                .ok_or_else(|| anyhow!("unknown join type"))?;
            let n = app.sheets.len();
            if n < 2 {
                bail!("need 2 sheets");
            }
            let cancel = CancelToken::new();
            let (a, b) = (&app.sheets[n - 1], &app.sheets[n - 2]);
            let joined =
                transform::join::join(&[a, b], kind, &cancel).map_err(|e| anyhow!(e))?;
            app.push_sheet(joined);
        }
        ChoosePurpose::Aggregator { col } => {
            let agg = Aggregator::from_name(choice).ok_or_else(|| anyhow!("unknown aggregator"))?;
            let Some(s) = app.sheet() else { return Ok(()) };
            let keys: Vec<usize> = {
                let mut k: Vec<usize> = (0..s.columns.len())
                    .filter(|&i| s.columns[i].is_key())
                    .collect();
                k.sort_by_key(|&i| s.columns[i].key_pos);
                k
            };
            // value column: first numeric non-key, non-pivot column; falls
            // back to the pivot column itself (count-style aggregation)
            let value_col = (0..s.columns.len())
                .find(|&i| {
                    !keys.contains(&i) && i != col && s.columns[i].vtype.is_numeric()
                })
                .unwrap_or(col);
            spawn_transform(app, "pivot", move |snap, cancel| {
                transform::pivot::pivot(snap, &keys, col, value_col, agg, cancel)
            });
        }
    }
    Ok(())
}

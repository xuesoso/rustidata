//! rustidata: a fast terminal tabular-data multitool (a Rust take on VisiData).

mod app;
mod canvas;
mod cli;
mod clipboard;
mod cliptext;
mod commands;
mod config;
mod draw_sheet;
mod input;
mod keys;
mod menu;
mod metasheets;
mod pager;
mod screen;
mod statusbar;
mod theme;

use rustidata_core::jobs::UiMsg;

fn main() -> anyhow::Result<()> {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Parsed::Run(c)) => c,
        Ok(cli::Parsed::Help) => {
            print!("{}", cli::help());
            return Ok(());
        }
        Ok(cli::Parsed::Version) => {
            println!("rustidata {}", cli::VERSION);
            return Ok(());
        }
        Err(e) => {
            eprintln!("rustidata: {}", e);
            std::process::exit(2);
        }
    };

    if cli.where_config {
        match config::config_path() {
            Some(p) => println!("{}", p.display()),
            None => println!("(no config directory)"),
        }
        return Ok(());
    }

    let config::Loaded {
        config: cfg,
        mut warnings,
    } = config::load();
    let mut opts = cfg.options.clone();
    if let Some(d) = &cli.delimiter {
        opts.delimiter = d.clone();
    }

    let theme_name = cli.theme.as_deref().unwrap_or(&cfg.theme.name);
    let mut theme = theme::theme_by_name(theme_name).unwrap_or_else(|| {
        warnings.push(format!("unknown theme '{}', using default", theme_name));
        theme::builtin_themes().remove(0)
    });
    config::apply_color_overrides(&mut theme, &cfg.colors);

    let (mut app, tx) = app::App::new(theme, opts);
    app.keybind_overrides = cfg.keybindings.clone();
    for w in &warnings {
        eprintln!("rustidata: {}", w); // visible if startup aborts pre-TUI
    }
    if let Some(w) = warnings.last() {
        app.status(rustidata_core::jobs::Severity::Warning, w.clone());
    }

    // open sources
    let notify = {
        let tx = tx.clone();
        std::sync::Arc::new(move || {
            let _ = tx.send(UiMsg::Dirty);
        })
    };
    if cli.files.is_empty() {
        // no args: stdin if piped, else current directory
        if stdin_is_pipe() {
            match rustidata_loaders::open_stdin(&app.opts, notify.clone()) {
                Ok(sheet) => app.push_sheet(sheet),
                Err(e) => {
                    eprintln!("rustidata: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            let cwd = std::env::current_dir()?;
            match rustidata_loaders::open_with_context(&cwd, &app.opts, notify.clone()) {
                Ok(sheet) => app.push_sheet(sheet),
                Err(e) => {
                    eprintln!("rustidata: {}", e);
                    std::process::exit(1);
                }
            }
        }
    } else {
        for f in &cli.files {
            if f.as_os_str() == "-" {
                match rustidata_loaders::open_stdin(&app.opts, notify.clone()) {
                    Ok(sheet) => app.push_sheet(sheet),
                    Err(e) => eprintln!("rustidata: stdin: {}", e),
                }
                continue;
            }
            match rustidata_loaders::open_with_context(f, &app.opts, notify.clone()) {
                Ok(sheet) => app.push_sheet(sheet),
                Err(e) => eprintln!("rustidata: {}: {}", f.display(), e),
            }
        }
    }
    if app.sheets.is_empty() {
        eprintln!("rustidata: nothing to open");
        std::process::exit(1);
    }

    app::run(&mut app)
}

fn stdin_is_pipe() -> bool {
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
}

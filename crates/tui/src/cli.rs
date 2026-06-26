//! Tiny hand-rolled argument parser (replaces clap for our handful of flags).
//! `parse` is pure and returns an outcome, so it is unit-testable; `main`
//! decides how to print help/version and what exit codes to use.

use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    /// Files to open (`-` for stdin).
    pub files: Vec<PathBuf>,
    /// Force a delimiter (e.g. `,` `;` `tab`).
    pub delimiter: Option<String>,
    /// Start with a specific theme.
    pub theme: Option<String>,
    /// Print the resolved config path and exit.
    pub where_config: bool,
}

/// Outcome of parsing: run with these args, or print help/version.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Cli),
    Help,
    Version,
}

/// Parse args (without the program name). Returns `Err(message)` on bad input.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cli = Cli::default();
    let mut it = args.into_iter();
    let mut rest_are_files = false;

    while let Some(arg) = it.next() {
        if rest_are_files {
            cli.files.push(PathBuf::from(arg));
            continue;
        }
        match arg.as_str() {
            "--" => rest_are_files = true,
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "--where-config" => cli.where_config = true,
            "-d" | "--delimiter" => cli.delimiter = Some(next_value(&mut it, &arg)?),
            "-t" | "--theme" => cli.theme = Some(next_value(&mut it, &arg)?),
            s if s.starts_with("--delimiter=") => {
                cli.delimiter = Some(s["--delimiter=".len()..].to_string());
            }
            s if s.starts_with("--theme=") => {
                cli.theme = Some(s["--theme=".len()..].to_string());
            }
            // attached short forms, e.g. `-d;` or `-tdark`
            s if s.starts_with("-d") => cli.delimiter = Some(s[2..].to_string()),
            s if s.starts_with("-t") => cli.theme = Some(s[2..].to_string()),
            // a lone "-" means stdin (a file); anything else dashed is unknown
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option '{}' (try --help)", s));
            }
            _ => cli.files.push(PathBuf::from(arg)),
        }
    }
    Ok(Parsed::Run(cli))
}

fn next_value<I: Iterator<Item = String>>(it: &mut I, flag: &str) -> Result<String, String> {
    it.next()
        .ok_or_else(|| format!("option '{}' requires a value", flag))
}

pub fn help() -> String {
    format!(
        "rustidata {VERSION}\n\
         Explore tabular data in the terminal: csv/tsv/json/parquet/sqlite/xlsx,\n\
         out-of-core for huge files, with visidata-style keybindings.\n\
         \n\
         USAGE:\n    \
         rustidata [OPTIONS] [FILES]...\n\
         \n\
         ARGS:\n    \
         <FILES>...    Files to open (- for stdin). Compressed (.gz/.bz2/.xz/.zst)\n                  \
         files are decompressed automatically; delimiters are sniffed.\n\
         \n\
         OPTIONS:\n    \
         -d, --delimiter <DELIM>    Force a delimiter (e.g. ',' ';' 'tab')\n    \
         -t, --theme <THEME>        Start with a specific theme\n        \
         --where-config         Print the resolved config path and exit\n    \
         -h, --help                 Print help\n    \
         -V, --version              Print version\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Cli {
        match parse(args.iter().map(|s| s.to_string())) {
            Ok(Parsed::Run(c)) => c,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn positional_files_and_stdin() {
        let c = run(&["a.csv", "-", "b.tsv"]);
        assert_eq!(
            c.files,
            vec![
                PathBuf::from("a.csv"),
                PathBuf::from("-"),
                PathBuf::from("b.tsv")
            ]
        );
    }

    #[test]
    fn delimiter_and_theme_all_forms() {
        assert_eq!(run(&["-d", ";"]).delimiter.as_deref(), Some(";"));
        assert_eq!(run(&["--delimiter", "tab"]).delimiter.as_deref(), Some("tab"));
        assert_eq!(run(&["--delimiter=,"]).delimiter.as_deref(), Some(","));
        assert_eq!(run(&["-d;"]).delimiter.as_deref(), Some(";"));
        assert_eq!(run(&["-t", "gruvbox"]).theme.as_deref(), Some("gruvbox"));
        assert_eq!(run(&["--theme=light"]).theme.as_deref(), Some("light"));
        assert_eq!(run(&["-tdark"]).theme.as_deref(), Some("dark"));
    }

    #[test]
    fn flags_and_mix() {
        let c = run(&["--where-config", "-d", ",", "data.csv"]);
        assert!(c.where_config);
        assert_eq!(c.delimiter.as_deref(), Some(","));
        assert_eq!(c.files, vec![PathBuf::from("data.csv")]);
    }

    #[test]
    fn double_dash_treats_rest_as_files() {
        let c = run(&["--", "-weird-name.csv", "--also"]);
        assert_eq!(
            c.files,
            vec![PathBuf::from("-weird-name.csv"), PathBuf::from("--also")]
        );
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(["--help".to_string()]), Ok(Parsed::Help));
        assert_eq!(parse(["-h".to_string()]), Ok(Parsed::Help));
        assert_eq!(parse(["--version".to_string()]), Ok(Parsed::Version));
        assert_eq!(parse(["-V".to_string()]), Ok(Parsed::Version));
    }

    #[test]
    fn unknown_option_errors_but_missing_value_too() {
        assert!(parse(["--nope".to_string()]).is_err());
        assert!(parse(["-d".to_string()]).is_err()); // requires a value
    }

    #[test]
    fn empty_is_ok() {
        assert_eq!(parse(std::iter::empty()), Ok(Parsed::Run(Cli::default())));
    }
}

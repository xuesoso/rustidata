//! User config at ~/.config/rustidata/config (TOML content, extensionless,
//! like git). Theme choice persists here; [colors] overrides individual
//! slots; [options] sets core options; [keybindings] remaps keys.

use std::path::PathBuf;

use anyhow::Result;
use rustidata_core::options::Options;
use serde::Deserialize;

use crate::theme::{parse_style, StyleKey, Theme};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ConfigFile {
    pub options: Options,
    pub theme: ThemeSection,
    pub colors: std::collections::HashMap<String, String>,
    pub keybindings: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ThemeSection {
    pub name: String,
}

impl Default for ThemeSection {
    fn default() -> Self {
        ThemeSection {
            name: "default-dark".into(),
        }
    }
}

/// Always ~/.config/rustidata (on every platform, per the documented
/// contract), with XDG_CONFIG_HOME respected when set.
pub fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("rustidata"));
        }
    }
    dirs::home_dir().map(|h| h.join(".config").join("rustidata"))
}

pub fn config_path() -> Option<PathBuf> {
    let dir = config_dir()?;
    let bare = dir.join("config");
    if bare.exists() {
        return Some(bare);
    }
    let toml = dir.join("config.toml");
    if toml.exists() {
        return Some(toml);
    }
    Some(bare) // default location for first write
}

/// A loaded config plus any problems found while parsing it. Warnings must be
/// surfaced in the UI: stderr is hidden as soon as the TUI takes the screen,
/// so a config error printed there is effectively invisible.
pub struct Loaded {
    pub config: ConfigFile,
    pub warnings: Vec<String>,
}

pub fn load() -> Loaded {
    let mut warnings = Vec::new();
    let config = match config_path() {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => parse_sections(&text, &path.display().to_string(), &mut warnings),
            Err(_) => ConfigFile::default(),
        },
        None => ConfigFile::default(),
    };
    Loaded { config, warnings }
}

/// Parse the config section by section, so one bad section (e.g. a hand-edited
/// [options] typo) doesn't silently discard the rest — like the persisted theme.
fn parse_sections(text: &str, origin: &str, warnings: &mut Vec<String>) -> ConfigFile {
    let mut cfg = ConfigFile::default();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("bad config {}: {}", origin, one_line(&e.to_string())));
            return cfg;
        }
    };
    if let Some(v) = table.get("options") {
        match v.clone().try_into() {
            Ok(o) => cfg.options = o,
            Err(e) => warnings.push(format!("config [options]: {}", one_line(&e.to_string()))),
        }
    }
    match table.get("theme") {
        // tolerate the shorthand `theme = "name"` form
        Some(toml::Value::String(name)) => cfg.theme.name = name.clone(),
        Some(v) => match v.clone().try_into() {
            Ok(t) => cfg.theme = t,
            Err(e) => warnings.push(format!("config [theme]: {}", one_line(&e.to_string()))),
        },
        None => {}
    }
    if let Some(v) = table.get("colors") {
        match v.clone().try_into() {
            Ok(c) => cfg.colors = c,
            Err(e) => warnings.push(format!("config [colors]: {}", one_line(&e.to_string()))),
        }
    }
    if let Some(v) = table.get("keybindings") {
        match v.clone().try_into() {
            Ok(k) => cfg.keybindings = k,
            Err(e) => warnings.push(format!("config [keybindings]: {}", one_line(&e.to_string()))),
        }
    }
    cfg
}

/// Squash a multi-line toml error (with span carets) onto one status line.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Apply [colors] overrides from the config onto a theme.
pub fn apply_color_overrides(theme: &mut Theme, colors: &std::collections::HashMap<String, String>) {
    for (name, spec) in colors {
        if let Some(key) = StyleKey::from_name(name) {
            theme.set(key, parse_style(spec));
        }
    }
}

/// Persist the theme name, preserving any other config content/comments.
pub fn persist_theme(name: &str) -> Result<()> {
    let Some(path) = config_path() else {
        anyhow::bail!("no config directory available");
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, upsert_theme_doc(&text, name))?;
    Ok(())
}

fn upsert_theme_doc(text: &str, name: &str) -> String {
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap_or_default();
    // replace a scalar `theme = "x"` (or missing section) with a proper table
    if !doc.get("theme").map_or(false, |i| i.is_table_like()) {
        doc["theme"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["theme"]["name"] = toml_edit::value(name);
    doc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let cfg: ConfigFile = toml::from_str(
            r#"
            [options]
            delimiter = ";"
            default_width = 25
            plot_dot_radius = 5

            [theme]
            name = "gruvbox"

            [colors]
            current_row = "bold white on 24"

            [keybindings]
            "zq" = "quit-sheet"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.options.delimiter, ";");
        assert_eq!(cfg.options.default_width, 25);
        assert_eq!(cfg.options.plot_dot_radius, 5);
        assert_eq!(cfg.theme.name, "gruvbox");
        assert_eq!(cfg.keybindings["zq"], "quit-sheet");
    }

    #[test]
    fn empty_config_defaults() {
        let cfg: ConfigFile = toml::from_str("").unwrap();
        assert_eq!(cfg.theme.name, "default-dark");
        assert_eq!(cfg.options.default_width, 20);
        assert_eq!(cfg.options.plot_dot_radius, 1);
    }

    #[test]
    fn bad_options_section_does_not_drop_theme() {
        let mut warnings = Vec::new();
        let cfg = parse_sections(
            "[options]\ndefault_width = \"oops\"\n\n[theme]\nname = \"gruvbox\"\n",
            "test",
            &mut warnings,
        );
        assert_eq!(cfg.theme.name, "gruvbox");
        assert_eq!(cfg.options.default_width, 20); // bad section falls back
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("[options]"));
    }

    #[test]
    fn top_level_theme_string_accepted() {
        let mut warnings = Vec::new();
        let cfg = parse_sections("theme = \"light\"\n", "test", &mut warnings);
        assert_eq!(cfg.theme.name, "light");
        assert!(warnings.is_empty());
    }

    #[test]
    fn persist_replaces_scalar_theme() {
        let out = upsert_theme_doc("theme = \"old\"\n[options]\ndelimiter = \",\"\n", "gruvbox-light");
        let cfg: ConfigFile = toml::from_str(&out).unwrap();
        assert_eq!(cfg.theme.name, "gruvbox-light");
        assert_eq!(cfg.options.delimiter, ",");
    }

    #[test]
    fn theme_edit_preserves_other_content() {
        let mut doc: toml_edit::DocumentMut = "# my config\n[options]\ndelimiter = \",\"\n"
            .parse()
            .unwrap();
        doc["theme"] = toml_edit::Item::Table(toml_edit::Table::new());
        doc["theme"]["name"] = toml_edit::value("light");
        let out = doc.to_string();
        assert!(out.contains("# my config"));
        assert!(out.contains("name = \"light\""));
        assert!(out.contains("delimiter"));
    }
}

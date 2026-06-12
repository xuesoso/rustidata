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

pub fn load() -> ConfigFile {
    let Some(path) = config_path() else {
        return ConfigFile::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("rustidata: bad config {}: {}", path.display(), e);
            ConfigFile::default()
        }),
        Err(_) => ConfigFile::default(),
    }
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
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap_or_default();
    if doc.get("theme").is_none() {
        doc["theme"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["theme"]["name"] = toml_edit::value(name);
    std::fs::write(&path, doc.to_string())?;
    Ok(())
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
        assert_eq!(cfg.theme.name, "gruvbox");
        assert_eq!(cfg.keybindings["zq"], "quit-sheet");
    }

    #[test]
    fn empty_config_defaults() {
        let cfg: ConfigFile = toml::from_str("").unwrap();
        assert_eq!(cfg.theme.name, "default-dark");
        assert_eq!(cfg.options.default_width, 20);
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

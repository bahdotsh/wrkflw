// User configuration for the wrkflw TUI.
//
// Reads `$XDG_CONFIG_HOME/wrkflw/config.toml` (falling back to
// `~/.config/wrkflw/config.toml`). Any parse error is surfaced as a warning
// on stderr and the defaults are used — config issues never prevent the
// app from starting.

use crate::theme::Theme;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Default, Deserialize)]
pub struct Config {
    /// `"dark"` (default) or `"light"`.
    pub theme: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Config::default();
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return Config::default();
        };
        match toml::from_str(&contents) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: failed to parse {}: {}", path.display(), e);
                Config::default()
            }
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("wrkflw").join("config.toml"))
}

/// Resolve the initial theme. Precedence: CLI `--theme` > config file > dark.
/// An unknown name warns and falls back to dark.
pub fn resolve_theme(config: &Config, cli_theme: Option<&str>) -> Theme {
    let name = cli_theme.or(config.theme.as_deref()).unwrap_or("dark");
    match Theme::by_name(name) {
        Some(t) => t,
        None => {
            eprintln!("Warning: unknown theme '{}', falling back to dark", name);
            Theme::dark()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_to_dark() {
        let t = resolve_theme(&Config::default(), None);
        assert!(t.is_dark);
    }

    #[test]
    fn resolve_cli_overrides_config() {
        let config = Config {
            theme: Some("dark".to_string()),
        };
        let t = resolve_theme(&config, Some("light"));
        assert!(!t.is_dark);
    }

    #[test]
    fn resolve_config_theme_when_no_cli() {
        let config = Config {
            theme: Some("light".to_string()),
        };
        let t = resolve_theme(&config, None);
        assert!(!t.is_dark);
    }

    #[test]
    fn resolve_unknown_falls_back_to_dark() {
        let t = resolve_theme(&Config::default(), Some("solarized"));
        assert!(t.is_dark);
    }
}

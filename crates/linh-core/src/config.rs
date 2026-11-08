use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Destination a URL can be routed to: a specific browser (optionally a
/// profile and/or a private/incognito window), or an explicit prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenIn {
    Target(Target),
    Prompt,
}

impl Serialize for OpenIn {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OpenIn::Prompt => serializer.serialize_str("prompt"),
            OpenIn::Target(target) => target.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for OpenIn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = toml::Value::deserialize(deserializer)?;
        match value {
            toml::Value::String(s) if s.eq_ignore_ascii_case("prompt") => Ok(OpenIn::Prompt),
            toml::Value::Table(_) => {
                let target: Target = value.try_into().map_err(serde::de::Error::custom)?;
                Ok(OpenIn::Target(target))
            }
            _ => Err(serde::de::Error::custom(
                "open_in must be the string \"prompt\" or a target table",
            )),
        }
    }
}

/// Where a URL should be opened: which browser, optionally which profile,
/// and whether it should be a private/incognito window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub browser: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default)]
    pub private: bool,
}

/// Whether the prompt window should always appear, or never (routing falls
/// through to rules / primary browser).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    #[default]
    Never,
    Always,
}

/// A single routing rule: an ordered list of match patterns (and, on
/// platforms that support it, a list of source apps) mapped to a
/// destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    #[serde(rename = "match")]
    pub match_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_apps: Option<Vec<String>>,
    pub open_in: OpenIn,
}

/// The full Linh configuration, as loaded from / saved to `linh.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub primary: Target,
    #[serde(default)]
    pub prompt_mode: PromptMode,
    #[serde(default)]
    pub shown_browsers: Vec<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            primary: Target {
                browser: "firefox".to_string(),
                profile: None,
                private: false,
            },
            prompt_mode: PromptMode::Never,
            shown_browsers: Vec::new(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

pub fn parse_config(input: &str) -> Result<Config, ConfigError> {
    toml::from_str(input).map_err(ConfigError::from)
}

pub fn serialize_config(config: &Config) -> Result<String, ConfigError> {
    toml::to_string_pretty(config).map_err(ConfigError::from)
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let data = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_config(&data)
}

pub fn save_config(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let data = serialize_config(config)?;
    fs::write(path, data).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips() {
        let config = Config::default();
        let toml_str = serialize_config(&config).expect("serialize");
        let parsed = parse_config(&toml_str).expect("parse");
        assert_eq!(config, parsed);
    }

    #[test]
    fn parses_rule_with_prompt_open_in() {
        let toml_str = r#"
            [primary]
            browser = "firefox"

            [[rules]]
            name = "always prompt"
            match = ["example.com"]
            open_in = "prompt"
        "#;
        let config = parse_config(toml_str).expect("parse");
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].open_in, OpenIn::Prompt);
    }

    #[test]
    fn parses_rule_with_target_open_in() {
        let toml_str = r#"
            [primary]
            browser = "firefox"

            [[rules]]
            name = "work"
            match = ["*.company.com"]

            [rules.open_in]
            browser = "chrome"
            profile = "Work"
            private = false
        "#;
        let config = parse_config(toml_str).expect("parse");
        assert_eq!(
            config.rules[0].open_in,
            OpenIn::Target(Target {
                browser: "chrome".to_string(),
                profile: Some("Work".to_string()),
                private: false,
            })
        );
    }

    #[test]
    fn rejects_invalid_open_in() {
        let toml_str = r#"
            [primary]
            browser = "firefox"

            [[rules]]
            name = "bad"
            match = ["example.com"]
            open_in = "not-prompt"
        "#;
        assert!(parse_config(toml_str).is_err());
    }

    #[test]
    fn load_and_save_round_trip_via_tempfile() {
        let dir = std::env::temp_dir().join(format!("linh-core-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("linh.toml");

        let config = Config::default();
        save_config(&path, &config).expect("save");
        let loaded = load_config(&path).expect("load");
        assert_eq!(config, loaded);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_errors() {
        let path = PathBuf::from("/nonexistent/linh-core-test/linh.toml");
        assert!(load_config(&path).is_err());
    }
}

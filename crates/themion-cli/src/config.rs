use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
struct FileConfig {
    api_key: Option<String>,
    model: Option<String>,
    system_prompt: Option<String>,
}

pub struct Config {
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
}

const DEFAULT_MODEL: &str = "minimax/minimax-m2.7";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful AI assistant with access to tools.";

const CONFIG_TEMPLATE: &str = r#"# themion config file
# Uncomment and edit to set defaults; env vars override these values.

# api_key = "sk-or-v1-..."
# model = "minimax/minimax-m2.7"
# system_prompt = "You are a helpful AI assistant with access to tools."
"#;

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("themion").join("config.toml"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let file_config = match config_path() {
            None => FileConfig::default(),
            Some(path) => {
                if !path.exists() {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("creating config dir {}", parent.display()))?;
                    }
                    fs::write(&path, CONFIG_TEMPLATE)
                        .with_context(|| format!("writing default config to {}", path.display()))?;
                    FileConfig::default()
                } else {
                    let raw = fs::read_to_string(&path)
                        .with_context(|| format!("reading config file {}", path.display()))?;
                    toml::from_str(&raw)
                        .with_context(|| format!("parsing config file {}", path.display()))?
                }
            }
        };

        // Env vars override file values.
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .or(file_config.api_key.filter(|s| !s.is_empty()));

        let model = std::env::var("OPENROUTER_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .or(file_config.model.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let system_prompt = std::env::var("SYSTEM_PROMPT")
            .ok()
            .filter(|s| !s.is_empty())
            .or(file_config.system_prompt.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        let api_key = api_key.ok_or_else(|| {
            let path_hint = config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/.config/themion/config.toml".to_string());
            anyhow::anyhow!(
                "api_key is required. Set OPENROUTER_API_KEY or add `api_key = \"...\"` to {}",
                path_hint
            )
        })?;

        Ok(Config { api_key, model, system_prompt })
    }
}

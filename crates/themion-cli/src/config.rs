use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Default, Clone)]
pub struct ProfileConfig {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Deserialize, Serialize, Default)]
struct FileConfig {
    primary_llm_profile: Option<String>,
    system_prompt: Option<String>,
    profile: Option<HashMap<String, ProfileConfig>>,
}

pub struct Config {
    pub active_profile: String,
    pub profiles: HashMap<String, ProfileConfig>,
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub system_prompt: String,
}

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful AI assistant with access to tools.";

const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_DEFAULT_MODEL: &str = "minimax/minimax-m2.7";

const LLAMACPP_DEFAULT_BASE_URL: &str = "http://localhost:8080/v1";
const LLAMACPP_DEFAULT_MODEL: &str = "local";

const CODEX_DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const CODEX_DEFAULT_MODEL: &str = "gpt-5.4";

const CONFIG_TEMPLATE: &str = r#"# themion config — https://github.com/you/themion
# primary_llm_profile selects which [profile.*] is active at startup.

primary_llm_profile = "default"

# system_prompt = "You are a helpful AI assistant with access to tools."

[profile.default]
provider = "openrouter"
# api_key = "sk-or-v1-..."
# model   = "minimax/minimax-m2.7"

# [profile.local]
# provider = "llamacpp"
# base_url = "http://localhost:8080/v1"
# model    = "local"
"#;

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("themion").join("config.toml"))
}

/// Resolve provider/base_url/api_key/model from a ProfileConfig, applying env overrides.
pub fn resolve_profile(profile: &ProfileConfig) -> (String, String, Option<String>, String) {
    let provider = std::env::var("THEMION_PROVIDER")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| profile.provider.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "openrouter".to_string());

    let (base_url, api_key, model) = match provider.as_str() {
        "llamacpp" => {
            let base_url = std::env::var("LLAMACPP_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.base_url.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| LLAMACPP_DEFAULT_BASE_URL.to_string());
            let model = std::env::var("LLAMACPP_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.model.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| LLAMACPP_DEFAULT_MODEL.to_string());
            (base_url, None, model)
        }
        "openai-codex" => {
            let base_url = profile
                .base_url
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| CODEX_DEFAULT_BASE_URL.to_string());
            let model = std::env::var("CODEX_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.model.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| CODEX_DEFAULT_MODEL.to_string());
            (base_url, None, model)
        }
        _ => {
            // Default: openrouter
            let base_url = std::env::var("OPENROUTER_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.base_url.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| OPENROUTER_DEFAULT_BASE_URL.to_string());
            let api_key = std::env::var("OPENROUTER_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.api_key.clone().filter(|s| !s.is_empty()));
            let model = std::env::var("OPENROUTER_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| profile.model.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| OPENROUTER_DEFAULT_MODEL.to_string());
            (base_url, api_key, model)
        }
    };

    (provider, base_url, api_key, model)
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

        // Determine active profile name: env > file > "default"
        let active_profile = std::env::var("THEMION_PROFILE")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| file_config.primary_llm_profile.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "default".to_string());

        let mut profiles = file_config.profile.unwrap_or_default();

        let profile = profiles.get(&active_profile).cloned().unwrap_or_default();

        let (provider, base_url, api_key, model) = resolve_profile(&profile);

        // Always ensure the active profile exists in the map so it appears in listings.
        // Use only file-level values here — never bake env-override values into the saved map.
        profiles
            .entry(active_profile.clone())
            .or_insert_with(|| ProfileConfig {
                provider: profile.provider.clone(),
                base_url: profile.base_url.clone(),
                model: profile.model.clone(),
                api_key: profile.api_key.clone(),
            });

        let system_prompt = std::env::var("SYSTEM_PROMPT")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| file_config.system_prompt.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        if provider == "openrouter" && api_key.is_none() {
            let path_hint = config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/.config/themion/config.toml".to_string());
            eprintln!(
                "error: api_key is required for provider=openrouter. \
                Set OPENROUTER_API_KEY or add `api_key = \"...\"` under [profile.{}] in {}",
                active_profile, path_hint
            );
            std::process::exit(1);
        }

        Ok(Config {
            active_profile,
            profiles,
            provider,
            base_url,
            api_key,
            model,
            system_prompt,
        })
    }
}

/// Persist profiles and active profile selection back to the config file.
pub fn save_profiles(
    active_profile: &str,
    profiles: &HashMap<String, ProfileConfig>,
) -> Result<()> {
    let path = match config_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    let raw = if path.exists() {
        fs::read_to_string(&path)
            .with_context(|| format!("reading config file {}", path.display()))?
    } else {
        String::new()
    };
    let mut file_config: FileConfig = toml::from_str(&raw).unwrap_or_default();
    file_config.primary_llm_profile = Some(active_profile.to_string());
    file_config.profile = Some(profiles.clone());
    let out = toml::to_string_pretty(&file_config).context("serializing config")?;
    fs::write(&path, out).with_context(|| format!("writing config file {}", path.display()))?;
    Ok(())
}

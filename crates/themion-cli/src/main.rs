mod auth_store;
mod config;
mod login_codex;
mod paste_burst;
mod tui;
use config::{Config, ProfileConfig};
use std::collections::HashMap;
use themion_core::agent::TurnStats;
use themion_core::ModelInfo;

pub fn format_duration(ms: u128) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn format_stats(s: &TurnStats) -> String {
    format!(
        "[stats: rounds={} tools={} in={} out={} cached={} time={}]",
        format_number(s.llm_rounds.into()),
        format_number(s.tool_calls.into()),
        format_number(s.tokens_in),
        format_number(s.tokens_out),
        format_number(s.tokens_cached),
        format_duration(s.elapsed_ms)
    )
}

#[derive(Clone)]
pub struct Session {
    pub active_profile: String,
    pub profiles: HashMap<String, ProfileConfig>,
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub system_prompt: String,
    pub model_info: Option<ModelInfo>,
}

impl Session {
    pub fn from_config(cfg: Config) -> Self {
        Self {
            active_profile: cfg.active_profile,
            profiles: cfg.profiles,
            provider: cfg.provider,
            base_url: cfg.base_url,
            api_key: cfg.api_key,
            model: cfg.model,
            system_prompt: cfg.system_prompt,
            model_info: None,
        }
    }

    pub fn switch_profile(&mut self, name: &str) -> bool {
        use crate::config::resolve_profile;
        let profile = match self.profiles.get(name) {
            Some(p) => p.clone(),
            None => return false,
        };
        let (provider, base_url, api_key, model) = resolve_profile(&profile);
        self.provider = provider;
        self.base_url = base_url;
        self.api_key = api_key;
        self.model = model;
        self.active_profile = name.to_string();
        self.model_info = None;
        true
    }
}

#[derive(Debug, PartialEq)]
enum EarlyExit {
    Version,
    Help,
    None,
}

fn handle_early_args(args: &[String]) -> EarlyExit {
    for arg in &args[1..] {
        match arg.as_str() {
            "--version" | "-V" => return EarlyExit::Version,
            "--help" | "-h" => return EarlyExit::Help,
            _ => {}
        }
    }
    EarlyExit::None
}

const HELP_TEXT: &str = "\
Usage: themion [OPTIONS] [PROMPT]

Options:
  --dir <DIR>     Project directory (default: current directory)
  --version, -V   Print version and exit
  --help, -h      Print this help and exit

When PROMPT is given, runs in print mode and exits.
Otherwise, launches the interactive TUI.

First run? Start the TUI and type /login codex.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match handle_early_args(&std::env::args().collect::<Vec<_>>()) {
        EarlyExit::Version => {
            println!("themion {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        EarlyExit::Help => {
            print!("{HELP_TEXT}");
            return Ok(());
        }
        EarlyExit::None => {}
    }

    let cfg = Config::load()?;

    let args: Vec<String> = std::env::args().skip(1).collect();

    let (project_dir_override, remaining_args) = {
        let mut dir: Option<std::path::PathBuf> = None;
        let mut rest = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--dir" {
                i += 1;
                if i < args.len() {
                    dir = Some(std::path::PathBuf::from(&args[i]));
                }
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
        (dir, rest)
    };

    if !remaining_args.is_empty() {
        let prompt = remaining_args.join(" ");

        let project_dir = project_dir_override
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
            .canonicalize()
            .unwrap_or_else(|_| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

        let db = match dirs::data_dir() {
            Some(d) => themion_core::db::open_default_in_data_dir(&d).unwrap_or_else(|_| {
                themion_core::db::DbHandle::open_in_memory().expect("in-memory db")
            }),
            None => themion_core::db::DbHandle::open_in_memory().expect("in-memory db"),
        };

        let session_id = uuid::Uuid::new_v4();
        let _ = db.insert_session(session_id, &project_dir, false);

        let client: Box<dyn themion_core::ChatBackend + Send + Sync> =
            if cfg.provider == "openai-codex" {
                let auth = auth_store::load()
                    .unwrap_or(None)
                    .ok_or_else(|| anyhow::anyhow!("no codex auth; run /login codex first"))?;
                Box::new(themion_core::client_codex::CodexClient::new(
                    cfg.base_url,
                    auth,
                    Box::new(|a: &themion_core::CodexAuth| auth_store::save(a)),
                ))
            } else {
                Box::new(themion_core::client::ChatClient::new(
                    cfg.base_url,
                    cfg.api_key,
                ))
            };
        let mut agent = themion_core::agent::Agent::new_with_db(
            client,
            cfg.model,
            cfg.system_prompt,
            session_id,
            project_dir,
            db,
        );
        agent.refresh_model_info().await;
        let (result, stats) = agent.run_loop(&prompt).await?;
        println!("{result}");
        eprintln!("{}", format_stats(&stats));
    } else {
        tui::run(cfg, project_dir_override).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_detected() {
        let args = vec!["themion".to_string(), "--version".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::Version);
    }

    #[test]
    fn version_short_flag_detected() {
        let args = vec!["themion".to_string(), "-V".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::Version);
    }

    #[test]
    fn help_flag_detected() {
        let args = vec!["themion".to_string(), "--help".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::Help);
    }

    #[test]
    fn help_short_flag_detected() {
        let args = vec!["themion".to_string(), "-h".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::Help);
    }

    #[test]
    fn no_early_exit() {
        let args = vec!["themion".to_string(), "some prompt".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::None);
    }

    #[test]
    fn version_flag_with_other_args() {
        let args = vec![
            "themion".to_string(),
            "--dir".to_string(),
            "/tmp".to_string(),
            "--version".to_string(),
        ];
        assert_eq!(handle_early_args(&args), EarlyExit::Version);
    }

    #[test]
    fn empty_args() {
        let args = vec!["themion".to_string()];
        assert_eq!(handle_early_args(&args), EarlyExit::None);
    }
}

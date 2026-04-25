mod auth_store;
mod config;
mod login_codex;
mod paste_burst;
mod app_runtime;
mod runtime_domains;
mod headless_runner;
#[cfg(feature = "stylos")]
mod stylos;
mod tui;
mod tui_runner;
use app_runtime::CliAppRuntime;
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
    pub id: uuid::Uuid,
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
            id: uuid::Uuid::new_v4(),
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

fn print_usage(program_name: &str) {
    println!(
        "Usage:
  {0}                     Start TUI mode
  {0} --headless          Start long-running headless mode
  {0} [--dir PATH] PROMPT Run one non-interactive prompt
  {0} --help              Show this help

Options:
  --dir PATH   Override project directory
  --headless   Start explicit long-running non-TUI mode
  --help       Show this help",
        program_name
    );
}

fn main() -> anyhow::Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    let program_name = raw_args
        .first()
        .cloned()
        .unwrap_or_else(|| "themion".to_string());
    let args: Vec<String> = raw_args.into_iter().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage(&program_name);
        return Ok(());
    }

    let cfg = Config::load()?;

    let (project_dir_override, headless_mode, remaining_args) = {
        let mut dir: Option<std::path::PathBuf> = None;
        let mut headless = false;
        let mut rest = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--dir" {
                i += 1;
                if i < args.len() {
                    dir = Some(std::path::PathBuf::from(&args[i]));
                }
            } else if args[i] == "--headless" {
                headless = true;
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
        (dir, headless, rest)
    };

    if headless_mode {
        if !remaining_args.is_empty() {
            anyhow::bail!("--headless does not accept prompt arguments; use prompt args for non-interactive mode");
        }
        let app_runtime = CliAppRuntime::for_headless(cfg, project_dir_override)?;
        let runtime_domains = app_runtime.runtime_domains.clone();
        runtime_domains.core().block_on(headless_runner::run(app_runtime))
    } else if !remaining_args.is_empty() {
        let app_runtime = CliAppRuntime::for_headless(cfg, project_dir_override)?;
        let runtime_domains = app_runtime.runtime_domains.clone();
        runtime_domains
            .core()
            .block_on(headless_runner::run_non_interactive(app_runtime, remaining_args.join(" ")))
    } else {
        let app_runtime = CliAppRuntime::for_tui(cfg, project_dir_override)?;
        let runtime_domains = app_runtime.runtime_domains.clone();
        let tui_runtime = runtime_domains
            .tui()
            .expect("tui runtime available in TUI mode");
        let result = tui_runtime.block_on(async move { tui_runner::run(app_runtime).await });
        drop(tui_runtime);
        drop(runtime_domains);
        result
    }
}

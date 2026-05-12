mod app_runtime;
mod app_state;
mod auth_store;
mod board_runtime;
mod build_info;
mod chat_composer;
mod config;
mod headless_runner;
mod instance_id;
mod local_prompts;
mod login_codex;
mod paste_burst;
mod runtime_domains;
mod source_analysis;
#[cfg(feature = "stylos")]
mod stylos;
mod surface_runner;
mod textarea;
mod tui;
mod tui_runner;
mod web;
mod web_assets;
mod web_terminal;
use app_state::AppState;
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
    pub configured_profile: String,
    pub active_profile: String,
    pub profiles: HashMap<String, ProfileConfig>,
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub effort: String,
    pub system_prompt: String,
    pub model_info: Option<ModelInfo>,
    pub temporary_profile_override: Option<String>,
    pub temporary_model_override: Option<String>,
    pub temporary_effort_override: Option<String>,
}

impl Session {
    pub fn from_config(cfg: Config) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            configured_profile: cfg.active_profile.clone(),
            active_profile: cfg.active_profile,
            profiles: cfg.profiles,
            provider: cfg.provider,
            base_url: cfg.base_url,
            api_key: cfg.api_key,
            model: cfg.model,
            effort: cfg.effort,
            system_prompt: cfg.system_prompt,
            model_info: None,
            temporary_profile_override: None,
            temporary_model_override: None,
            temporary_effort_override: None,
        }
    }

    pub fn switch_profile(&mut self, name: &str) -> bool {
        use crate::config::resolve_profile;
        let profile = match self.profiles.get(name) {
            Some(p) => p.clone(),
            None => return false,
        };
        let (provider, base_url, api_key, mut model, mut effort) = resolve_profile(&profile);
        if let Some(temporary_model_override) = &self.temporary_model_override {
            model = temporary_model_override.clone();
        }
        if let Some(temporary_effort_override) = &self.temporary_effort_override {
            effort = temporary_effort_override.clone();
        }
        self.provider = provider;
        self.base_url = base_url;
        self.api_key = api_key;
        self.model = model;
        self.effort = effort;
        self.active_profile = name.to_string();
        self.model_info = None;
        true
    }

    pub fn switch_profile_temporarily(&mut self, name: &str) -> bool {
        self.temporary_profile_override = Some(name.to_string());
        self.temporary_model_override = None;
        self.temporary_effort_override = None;
        self.switch_profile(name)
    }

    pub fn set_temporary_model_override(&mut self, model: &str) {
        self.temporary_model_override = Some(model.to_string());
        self.model = model.to_string();
        self.model_info = None;
    }

    pub fn set_temporary_effort_override(&mut self, effort: &str) {
        self.temporary_effort_override = Some(effort.to_string());
        self.effort = effort.to_string();
        self.model_info = None;
    }

    pub fn clear_temporary_overrides(&mut self) -> bool {
        let had_overrides = self.temporary_profile_override.is_some()
            || self.temporary_model_override.is_some()
            || self.temporary_effort_override.is_some();
        self.temporary_profile_override = None;
        self.temporary_model_override = None;
        self.temporary_effort_override = None;
        let configured_profile = self.configured_profile.clone();
        let switched = self.switch_profile(&configured_profile);
        had_overrides && switched
    }
}

fn print_usage(program_name: &str) {
    println!(
        "Usage:
  {0}                                Start TUI mode
  {0} --headless                     Start long-running headless mode
  {0} --web [--bind ADDR]            Start minimal web mode
  {0} [--dir PATH] PROMPT            Run one non-interactive prompt
  {0} --command unified-search index [--full] [--source-kind KIND] [--dir PATH]
                                    Build or rebuild generalized unified-search indexes for the selected project or one source kind
  {0} --help                         Show this help

Options:
  --dir PATH    Override project directory
  --headless    Start explicit long-running non-TUI mode
  --web         Start minimal web mode
  --bind ADDR   Override web bind address (default 127.0.0.1:8420)
  --command     Run an explicit non-prompt CLI command
  --full        Rebuild generalized unified-search indexes for the selected project
  --source-kind Limit unified-search indexing to one source kind: memory, chat_message, tool_call, or tool_result
  --help        Show this help",
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
    let startup_banner = crate::build_info::BuildIdentity::current().startup_banner_text();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{startup_banner}");
        print_usage(&program_name);
        return Ok(());
    }

    let cfg = Config::load()?;

    let (
        project_dir_override,
        headless_mode,
        web_mode,
        web_bind_addr,
        command_mode,
        remaining_args,
    ) = {
        let mut dir: Option<std::path::PathBuf> = None;
        let mut headless = false;
        let mut web = false;
        let mut web_bind_addr: Option<String> = None;
        let mut command_mode = false;
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
            } else if args[i] == "--web" {
                web = true;
            } else if args[i] == "--bind" {
                i += 1;
                if i < args.len() {
                    web_bind_addr = Some(args[i].clone());
                }
            } else if args[i] == "--command" {
                command_mode = true;
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
        (dir, headless, web, web_bind_addr, command_mode, rest)
    };

    if command_mode {
        println!("{startup_banner}");
        if headless_mode {
            anyhow::bail!("--command cannot be combined with --headless");
        }
        if web_mode {
            anyhow::bail!("--command cannot be combined with --web");
        }
        if web_bind_addr.is_some() {
            anyhow::bail!("--bind requires --web");
        }
        if let Some((force_full, source_kind, rest_after_command)) =
            parse_unified_search_index_command(&remaining_args)
        {
            if !rest_after_command.is_empty() {
                anyhow::bail!(
                    "unified-search index does not accept extra arguments beyond --full and --source-kind <kind>"
                );
            }
            #[cfg(not(feature = "semantic-memory"))]
            {
                let _ = force_full;
                let _ = &source_kind;
                anyhow::bail!(
                    "unified-search index requires building themion-cli with the semantic-memory feature"
                );
            }
            #[cfg(feature = "semantic-memory")]
            {
                let app_runtime = AppState::for_headless(cfg, project_dir_override)?;
                let runtime_domains = app_runtime.runtime_domains.clone();
                return runtime_domains
                    .background()
                    .expect("background runtime available in headless mode")
                    .block_on(headless_runner::run_unified_search_index(
                        app_runtime,
                        force_full,
                        source_kind,
                    ));
            }
        }
        anyhow::bail!(
            "unknown command '{}'. Use --command unified-search index [--full] [--source-kind <kind>]",
            remaining_args.join(" ")
        );
    }

    if headless_mode && web_mode {
        anyhow::bail!("--headless cannot be combined with --web");
    }
    if web_bind_addr.is_some() && !web_mode {
        anyhow::bail!("--bind requires --web");
    }

    if headless_mode {
        println!("{startup_banner}");
        if !remaining_args.is_empty() {
            anyhow::bail!("--headless does not accept prompt arguments; use prompt args for non-interactive mode");
        }
        let app_runtime = AppState::for_headless(cfg, project_dir_override)?;
        let runtime_domains = app_runtime.runtime_domains.clone();
        runtime_domains
            .core()
            .block_on(headless_runner::run(app_runtime))
    } else if web_mode {
        println!("{startup_banner}");
        if !remaining_args.is_empty() {
            anyhow::bail!(
                "--web does not accept prompt arguments; use prompt args for non-interactive mode"
            );
        }
        let bind_addr = web::parse_bind_addr(web_bind_addr.as_deref())?;
        let app_runtime = AppState::for_headless(cfg, project_dir_override)?;
        web::run(app_runtime, bind_addr)
    } else if !remaining_args.is_empty() {
        println!("{startup_banner}");
        let app_runtime = AppState::for_headless(cfg, project_dir_override)?;
        let runtime_domains = app_runtime.runtime_domains.clone();
        runtime_domains
            .core()
            .block_on(headless_runner::run_non_interactive(
                app_runtime,
                remaining_args.join(" "),
            ))
    } else {
        let app_runtime = AppState::for_tui(cfg, project_dir_override)?;
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

fn is_valid_unified_search_source_kind(value: &str) -> bool {
    matches!(
        value,
        "memory" | "chat_message" | "tool_call" | "tool_result"
    )
}

fn parse_unified_search_index_command(
    args: &[String],
) -> Option<(bool, Option<String>, Vec<String>)> {
    let [domain, command, rest @ ..] = args else {
        return None;
    };
    if domain != "unified-search" || command != "index" {
        return None;
    }
    let mut force_full = false;
    let mut source_kind: Option<String> = None;
    let mut trailing = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--full" {
            force_full = true;
            i += 1;
        } else if rest[i] == "--source-kind" {
            i += 1;
            if i >= rest.len() {
                trailing.push("--source-kind".to_string());
                break;
            }
            let value = rest[i].clone();
            if !is_valid_unified_search_source_kind(&value) || source_kind.is_some() {
                trailing.push("--source-kind".to_string());
                trailing.push(value);
            } else {
                source_kind = Some(value);
            }
            i += 1;
        } else {
            trailing.push(rest[i].clone());
            i += 1;
        }
    }
    Some((force_full, source_kind, trailing))
}

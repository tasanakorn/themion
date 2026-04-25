use crate::config::Config;
use crate::runtime_domains::RuntimeDomains;
use crate::Session;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::agent::Agent;
use themion_core::client::ChatClient;
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
use themion_core::tools::{
    SystemInspectionProvider, SystemInspectionResult, SystemInspectionRuntime,
    SystemInspectionTools,
};
use themion_core::ChatBackend;
use uuid::Uuid;

pub struct CliAppRuntime {
    pub runtime_domains: Arc<RuntimeDomains>,
    pub session: Session,
    pub db: Arc<DbHandle>,
    pub project_dir: PathBuf,
    pub session_id: Uuid,
    #[cfg(feature = "stylos")]
    pub stylos_config: crate::config::StylosConfig,
}

impl CliAppRuntime {
    pub fn for_tui(cfg: Config, project_dir_override: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::build(cfg, project_dir_override, true)
    }

    pub fn for_headless(
        cfg: Config,
        project_dir_override: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::build(cfg, project_dir_override, false)
    }

    fn build(
        cfg: Config,
        project_dir_override: Option<PathBuf>,
        interactive: bool,
    ) -> anyhow::Result<Self> {
        #[cfg(feature = "stylos")]
        let stylos_config = cfg.stylos.clone();

        let runtime_domains = Arc::new(if interactive {
            RuntimeDomains::for_tui_mode()?
        } else {
            RuntimeDomains::for_print_mode()?
        });
        let project_dir = resolve_project_dir(project_dir_override);
        let db = open_history_db(interactive);
        let session = Session::from_config(cfg);
        let session_id = Uuid::new_v4();
        let _ = db.insert_session(session_id, &project_dir, interactive);

        Ok(Self {
            runtime_domains,
            session,
            db,
            project_dir,
            session_id,
            #[cfg(feature = "stylos")]
            stylos_config,
        })
    }

    pub fn system_inspection_snapshot(&self) -> SystemInspectionResult {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut provider = SystemInspectionProvider {
            status: "ok".to_string(),
            active_profile: Some(self.session.active_profile.clone()),
            provider: Some(self.session.provider.clone()),
            model: Some(self.session.model.clone()),
            auth_configured: Some(match self.session.provider.as_str() {
                "openai-codex" => crate::auth_store::load().ok().flatten().is_some(),
                _ => self
                    .session
                    .api_key
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            }),
            base_url_present: Some(!self.session.base_url.trim().is_empty()),
            rate_limits: None,
            warnings: Vec::new(),
            issues: Vec::new(),
        };
        if provider.auth_configured == Some(false) {
            provider.status = "degraded".to_string();
            provider
                .issues
                .push("provider authentication is not configured".to_string());
        }
        if provider.base_url_present == Some(false) {
            provider.status = "degraded".to_string();
            provider
                .issues
                .push("provider base_url is empty".to_string());
        }

        let tool_names = themion_core::tools::tool_definitions()
            .as_array()
            .into_iter()
            .flat_map(|defs| defs.iter())
            .filter_map(|entry| entry.get("function")?.get("name")?.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();

        let tools = SystemInspectionTools {
            status: "ok".to_string(),
            tool_count: tool_names.len(),
            available_names: tool_names,
            warnings: Vec::new(),
            issues: Vec::new(),
        };

        let runtime = SystemInspectionRuntime {
            status: "ok".to_string(),
            pid: Some(std::process::id()),
            now_ms,
            session_id: self.session_id.to_string(),
            project_dir: self.project_dir.display().to_string(),
            workflow_name: None,
            phase_name: None,
            workflow_status: None,
            debug_runtime_lines: vec![
                "debug runtime snapshot unavailable outside the TUI app loop".to_string(),
            ],
            warnings: vec!["runtime inspection is partial outside the TUI app loop".to_string()],
            issues: Vec::new(),
        };

        let mut warnings = Vec::new();
        let mut issues = Vec::new();
        if provider.status != "ok" {
            warnings.push("provider readiness is degraded".to_string());
            issues.extend(provider.issues.clone());
        }
        let overall_status = if issues.is_empty() { "ok" } else { "degraded" }.to_string();
        let summary = if overall_status == "ok" {
            "local inspection snapshot available".to_string()
        } else {
            format!("local inspection found {} issue(s)", issues.len())
        };

        SystemInspectionResult {
            overall_status,
            summary,
            runtime,
            tools,
            provider,
            warnings,
            issues,
        }
    }

    pub fn build_agent(&self) -> anyhow::Result<Agent> {
        build_agent(
            &self.session,
            self.session_id,
            self.project_dir.clone(),
            self.db.clone(),
            #[cfg(feature = "stylos")]
            None,
            #[cfg(feature = "stylos")]
            None,
            #[cfg(feature = "stylos")]
            "main",
            Some(self.system_inspection_snapshot()),
        )
    }
}

pub fn resolve_project_dir(project_dir_override: Option<PathBuf>) -> PathBuf {
    project_dir_override
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn open_history_db(interactive: bool) -> Arc<DbHandle> {
    match dirs::data_dir() {
        Some(d) => themion_core::db::open_default_in_data_dir(&d).unwrap_or_else(|e| {
            if interactive {
                eprintln!("warning: history persistence disabled: {}", e);
            }
            DbHandle::open_in_memory().expect("in-memory db")
        }),
        None => {
            if interactive {
                eprintln!("warning: history persistence disabled (no data dir)");
            }
            DbHandle::open_in_memory().expect("in-memory db")
        }
    }
}

#[cfg(feature = "stylos")]
pub async fn start_stylos(
    app_runtime: &CliAppRuntime,
) -> anyhow::Result<crate::stylos::StylosHandle> {
    match app_runtime
        .runtime_domains
        .network()
        .spawn({
            let stylos_cfg = app_runtime.stylos_config.clone();
            let session = app_runtime.session.clone();
            let project_dir = app_runtime.project_dir.clone();
            let db = app_runtime.db.clone();
            let network_domain = app_runtime.runtime_domains.network();
            async move {
                crate::stylos::start(&stylos_cfg, &session, &project_dir, db, network_domain).await
            }
        })
        .await
    {
        Ok(handle) => Ok(handle),
        Err(err) => Err(anyhow::anyhow!("failed to start stylos runtime: {}", err)),
    }
}

pub fn build_agent(
    session: &Session,
    session_id: Uuid,
    project_dir: PathBuf,
    db: Arc<DbHandle>,
    #[cfg(feature = "stylos")] stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")] local_instance_id: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: &str,
    system_inspection: Option<SystemInspectionResult>,
) -> anyhow::Result<Agent> {
    let client: Box<dyn ChatBackend + Send + Sync> = match session.provider.as_str() {
        "openai-codex" => {
            let auth = crate::auth_store::load()?
                .ok_or_else(|| anyhow::anyhow!("no codex auth; run /login codex first"))?;
            Box::new(CodexClient::new(
                session.base_url.clone(),
                auth,
                Box::new(|a: &themion_core::CodexAuth| crate::auth_store::save(a)),
            ))
        }
        _ => {
            let mut c = ChatClient::new(session.base_url.clone(), session.api_key.clone());
            if session.provider == "openrouter" {
                c = c.with_headers([
                    (
                        "HTTP-Referer".to_string(),
                        "https://github.com/tasanakorn".to_string(),
                    ),
                    ("X-Title".to_string(), "themion".to_string()),
                    ("X-OpenRouter-Title".to_string(), "themion".to_string()),
                    (
                        "X-OpenRouter-Categories".to_string(),
                        "developer-tools".to_string(),
                    ),
                ]);
            }
            Box::new(c)
        }
    };
    #[cfg(feature = "stylos")]
    let mut agent = Agent::new_with_db(
        client,
        session.model.clone(),
        session.system_prompt.clone(),
        session_id,
        project_dir,
        db,
    );
    #[cfg(not(feature = "stylos"))]
    let mut agent = Agent::new_with_db(
        client,
        session.model.clone(),
        session.system_prompt.clone(),
        session_id,
        project_dir,
        db,
    );
    agent.set_system_inspection(system_inspection);
    #[cfg(feature = "stylos")]
    agent.set_local_agent_id(Some(local_agent_id.to_string()));
    #[cfg(feature = "stylos")]
    agent.set_local_instance_id(local_instance_id.map(str::to_string));
    #[cfg(feature = "stylos")]
    agent.set_stylos_tool_invoker(crate::tui::stylos_tool_invoker(stylos_tool_bridge));
    Ok(agent)
}

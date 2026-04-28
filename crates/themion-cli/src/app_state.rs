use crate::config::Config;
use crate::runtime_domains::RuntimeDomains;
use crate::Session;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::agent::Agent;
use themion_core::client::ChatClient;
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
#[cfg(feature = "stylos")]
use themion_core::db::{CreateNoteArgs, NoteColumn, NoteKind};
use themion_core::tools::{
    SystemInspectionProvider, SystemInspectionResult, SystemInspectionRuntime,
    SystemInspectionTaskRuntime, SystemInspectionTools,
};
use themion_core::ChatBackend;
use uuid::Uuid;

pub struct AppState {
    pub runtime_domains: Arc<RuntimeDomains>,
    pub session: Session,
    pub db: Arc<DbHandle>,
    pub project_dir: PathBuf,
    pub session_id: Uuid,
    #[cfg(feature = "stylos")]
    pub stylos_config: crate::config::StylosConfig,
}

#[cfg(feature = "stylos")]
pub struct DoneMentionRequest {
    pub note_id: String,
    pub note_slug: String,
    pub from_instance: String,
    pub from_agent_id: String,
    pub completed_by_instance: String,
    pub completed_by_agent_id: String,
    pub result_summary: String,
}

impl AppState {
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
            task_runtime: Some(SystemInspectionTaskRuntime {
                status: "partial".to_string(),
                current_activity: None,
                current_activity_detail: None,
                busy: None,
                activity_status: None,
                activity_status_changed_at_ms: None,
                process_started_at_ms: None,
                uptime_ms: None,
                recent_window_ms: None,
                runtime_notes: vec![
                    "task runtime inspection is unavailable outside the TUI app loop".to_string(),
                ],
            }),
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
            false,
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
pub async fn start_stylos(app_state: &AppState) -> anyhow::Result<crate::stylos::StylosHandle> {
    match app_state
        .runtime_domains
        .network()
        .spawn({
            let stylos_cfg = app_state.stylos_config.clone();
            let session = app_state.session.clone();
            let project_dir = app_state.project_dir.clone();
            let db = app_state.db.clone();
            let network_domain = app_state.runtime_domains.network();
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

#[cfg(feature = "stylos")]
#[cfg(feature = "stylos")]
#[cfg(feature = "stylos")]
pub fn create_done_mention_locally(
    db: &DbHandle,
    request: &DoneMentionRequest,
) -> anyhow::Result<String> {
    let body = format!(
        "Done: delegated note completed.\n\nOriginal note: {} ({})\nCompleted by: {} / {}\nResult:\n{}",
        request.note_id,
        request.note_slug,
        request.completed_by_instance,
        request.completed_by_agent_id,
        request.result_summary,
    );
    db.create_board_note(CreateNoteArgs {
        note_id: uuid::Uuid::new_v4().to_string(),
        note_kind: NoteKind::DoneMention,
        column: NoteColumn::Todo,
        origin_note_id: Some(request.note_id.clone()),
        from_instance: Some(request.completed_by_instance.clone()),
        from_agent_id: Some(request.completed_by_agent_id.clone()),
        to_instance: request.from_instance.clone(),
        to_agent_id: request.from_agent_id.clone(),
        body,
        meta_json: None,
    })
    .map(|done_note| {
        serde_json::json!({
            "accepted": true,
            "note_id": done_note.note_id,
            "note_slug": done_note.note_slug,
            "agent_id": done_note.to_agent_id,
        })
        .to_string()
    })
    .map_err(anyhow::Error::from)
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
    api_log_enabled: bool,
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

    let mut agent = Agent::new_with_db(
        client,
        session.model.clone(),
        Some(session.provider.clone()),
        Some(session.active_profile.clone()),
        session.system_prompt.clone(),
        session_id,
        project_dir,
        db,
    );
    agent.set_api_log_enabled(api_log_enabled);
    agent.set_system_inspection(system_inspection);

    #[cfg(feature = "stylos")]
    {
        agent.set_stylos_tool_invoker(crate::tui::stylos_tool_invoker(stylos_tool_bridge));
        agent.set_local_instance_id(local_instance_id.map(str::to_string));
        agent.set_local_agent_id(Some(local_agent_id.to_string()));
    }

    Ok(agent)
}

use std::path::PathBuf;
use std::sync::Arc;

use themion_core::agent::Agent;
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::db::DbHandle;
use themion_core::tools::{
    SystemInspectionProvider, SystemInspectionRateLimits, SystemInspectionResult,
    SystemInspectionRuntime, SystemInspectionTaskRuntime, SystemInspectionTools,
};
use themion_core::workflow::WorkflowState;
use uuid::Uuid;

use crate::Session;

pub(crate) fn build_system_inspection_snapshot(
    session: &Session,
    fallback_session_id: Uuid,
    interactive_session_id: Option<Uuid>,
    project_dir: &std::path::Path,
    workflow_state: &WorkflowState,
    rate_limits: Option<&ApiCallRateLimitReport>,
    task_runtime: SystemInspectionTaskRuntime,
    debug_runtime_lines: Vec<String>,
) -> SystemInspectionResult {
    let rate_limits = rate_limits.map(|report| SystemInspectionRateLimits {
        api_call: report.api_call.clone(),
        source: report.source.clone(),
        http_status: report.http_status,
        active_limit: report.active_limit.clone(),
        snapshot_count: report.snapshots.len(),
    });
    let mut provider = SystemInspectionProvider {
        status: "ok".to_string(),
        active_profile: Some(session.active_profile.clone()),
        provider: Some(session.provider.clone()),
        model: Some(session.model.clone()),
        auth_configured: Some(match session.provider.as_str() {
            "openai-codex" => crate::auth_store::load().ok().flatten().is_some(),
            _ => session
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
        }),
        base_url_present: Some(!session.base_url.trim().is_empty()),
        rate_limits,
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
        now_ms: unix_epoch_now_ms(),
        session_id: interactive_session_id
            .unwrap_or(fallback_session_id)
            .to_string(),
        project_dir: project_dir.display().to_string(),
        workflow_name: Some(workflow_state.workflow_name.clone()),
        phase_name: Some(workflow_state.phase_name.clone()),
        workflow_status: Some(format!("{:?}", workflow_state.status)),
        debug_runtime_lines,
        task_runtime: Some(task_runtime),
        warnings: Vec::new(),
        issues: Vec::new(),
    };
    let mut warnings = Vec::new();
    let issues = provider.issues.clone();
    if provider.status != "ok" {
        warnings.push("provider readiness is degraded".to_string());
    }
    let overall_status = if issues.is_empty() { "ok" } else { "degraded" }.to_string();
    let summary = if overall_status == "ok" {
        "local inspection snapshot available, including /debug runtime coverage".to_string()
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

pub(crate) struct AgentReplacementParams<'a> {
    pub session: &'a Session,
    pub project_dir: &'a PathBuf,
    pub db: &'a Arc<DbHandle>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")]
    pub local_stylos_instance: Option<&'a str>,
    pub api_log_enabled: bool,
    pub insert_session: bool,
}

pub(crate) fn build_replacement_main_agent(
    params: AgentReplacementParams<'_>,
) -> anyhow::Result<(Agent, Uuid)> {
    let new_session_id = Uuid::new_v4();
    let new_agent = crate::app_state::build_agent(
        params.session,
        new_session_id,
        params.project_dir.clone(),
        params.db.clone(),
        #[cfg(feature = "stylos")]
        params.stylos_tool_bridge,
        #[cfg(feature = "stylos")]
        params.local_stylos_instance,
        #[cfg(feature = "stylos")]
        "master",
        None,
        params.api_log_enabled,
    )?;
    if params.insert_session {
        let _ = params
            .db
            .insert_session(new_session_id, params.project_dir, true);
    }
    Ok((new_agent, new_session_id))
}

fn unix_epoch_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

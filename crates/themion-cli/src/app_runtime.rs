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
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::tui::{AppEvent, LocalAgentManagementRequest};
use crate::Session;

pub(crate) fn build_local_agent_tool_invoker(
    app_tx: mpsc::UnboundedSender<AppEvent>,
) -> themion_core::tools::LocalAgentToolInvoker {
    std::sync::Arc::new(move |name: String, args: serde_json::Value| {
        let app_tx = app_tx.clone();
        let fut: std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
        > = Box::pin(async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            app_tx
                .send(AppEvent::LocalAgentManagement(
                    LocalAgentManagementRequest {
                        action: name,
                        args,
                        reply_tx,
                    },
                ))
                .map_err(|_| anyhow::anyhow!("local agent management queue unavailable"))?;
            reply_rx
                .await
                .map_err(|_| anyhow::anyhow!("local agent management reply unavailable"))?
        });
        fut
    })
}

pub(crate) fn build_main_agent(
    session: &Session,
    db: Arc<DbHandle>,
    session_id: Uuid,
    project_dir: PathBuf,
    local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
    #[cfg(feature = "stylos")] stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")] local_stylos_instance: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: &str,
    system_inspection: Option<SystemInspectionResult>,
    api_log_enabled: bool,
) -> anyhow::Result<Agent> {
    crate::app_state::build_agent(
        session,
        session_id,
        project_dir,
        db,
        #[cfg(feature = "stylos")]
        stylos_tool_bridge,
        #[cfg(feature = "stylos")]
        local_stylos_instance,
        #[cfg(feature = "stylos")]
        local_agent_id,
        Some(build_local_agent_tool_invoker(local_agent_mgmt_tx)),
        system_inspection,
        api_log_enabled,
    )
}

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
    pub local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
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
    let new_agent = build_main_agent(
        params.session,
        params.db.clone(),
        new_session_id,
        params.project_dir.clone(),
        params.local_agent_mgmt_tx,
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

pub(crate) struct LocalAgentRuntimeContext<'a> {
    pub session: &'a Session,
    pub project_dir: &'a PathBuf,
    pub db: &'a Arc<DbHandle>,
    pub agents: &'a mut Vec<crate::tui::AgentHandle>,
    pub agent_busy: bool,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")]
    pub local_stylos_instance: Option<&'a str>,
    pub local_agent_tool_invoker: Option<themion_core::tools::LocalAgentToolInvoker>,
    pub api_log_enabled: bool,
}

fn normalize_primary_role(value: &str) -> &str {
    if value == "main" {
        "master"
    } else {
        value
    }
}

fn normalize_role_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut roles = value
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    roles.sort();
    roles.dedup();
    roles
}

fn allocate_default_local_agent_id(agents: &[crate::tui::AgentHandle]) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("smith-{n}");
        if !agents.iter().any(|h| h.agent_id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn is_interactive_handle(handle: &crate::tui::AgentHandle) -> bool {
    handle.roles.iter().any(|r| r == "interactive")
}

pub(crate) fn handle_local_agent_management_request(
    ctx: LocalAgentRuntimeContext<'_>,
    action: &str,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    match action {
        "local_agent_create" => create_local_agent(ctx, args),
        "local_agent_delete" => delete_local_agent(ctx, args),
        other => Err(anyhow::anyhow!(
            "unknown local agent management action: {other}"
        )),
    }
}

fn create_local_agent(
    ctx: LocalAgentRuntimeContext<'_>,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    let requested_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let agent_id = requested_id
        .map(str::to_string)
        .unwrap_or_else(|| allocate_default_local_agent_id(ctx.agents));
    if agent_id == "master" {
        anyhow::bail!("agent_id 'master' is reserved for the predefined leader");
    }
    if ctx.agents.iter().any(|h| h.agent_id == agent_id) {
        anyhow::bail!("duplicate agent_id: {agent_id}");
    }
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(agent_id.as_str())
        .to_string();
    let roles = normalize_role_list(args.get("roles"));
    if roles.iter().any(|r| normalize_primary_role(r) == "master") {
        anyhow::bail!("cannot create another master agent");
    }
    if roles.iter().any(|r| r == "interactive") && ctx.agents.iter().any(is_interactive_handle) {
        anyhow::bail!("invalid agent roles: expected at most one interactive agent");
    }
    let session_id = Uuid::new_v4();
    let _ = ctx.db.insert_session(session_id, ctx.project_dir, true);
    let agent = crate::app_state::build_agent(
        ctx.session,
        session_id,
        ctx.project_dir.clone(),
        ctx.db.clone(),
        #[cfg(feature = "stylos")]
        ctx.stylos_tool_bridge.clone(),
        #[cfg(feature = "stylos")]
        ctx.local_stylos_instance,
        #[cfg(feature = "stylos")]
        &agent_id,
        ctx.local_agent_tool_invoker,
        None,
        ctx.api_log_enabled,
    )?;
    ctx.agents.push(crate::tui::AgentHandle {
        agent: Some(agent),
        session_id,
        agent_id: agent_id.clone(),
        label: label.clone(),
        roles: roles.clone(),
        busy: false,
        #[cfg(feature = "stylos")]
        active_incoming_prompt: None,
    });
    Ok(serde_json::json!({
        "ok": true,
        "entity": "local_agent",
        "operation": "create",
        "agent_id": agent_id,
        "label": label,
        "roles": roles,
        "session_id": session_id.to_string(),
    })
    .to_string())
}

fn delete_local_agent(
    ctx: LocalAgentRuntimeContext<'_>,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
    if agent_id == "master" {
        anyhow::bail!("cannot delete the predefined leader agent");
    }
    if ctx.agent_busy {
        anyhow::bail!("cannot delete local agents while the local runtime is busy");
    }
    let index = ctx
        .agents
        .iter()
        .position(|h| h.agent_id == agent_id)
        .ok_or_else(|| anyhow::anyhow!("unknown agent_id: {agent_id}"))?;
    let removed = ctx.agents.remove(index);
    Ok(serde_json::json!({
        "ok": true,
        "entity": "local_agent",
        "operation": "delete",
        "agent_id": removed.agent_id,
        "label": removed.label,
        "session_id": removed.session_id.to_string(),
    })
    .to_string())
}

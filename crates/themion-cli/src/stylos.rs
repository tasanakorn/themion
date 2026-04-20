#![cfg(feature = "stylos")]

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use stylos_config::{Endpoints, IdentitySection, StylosConfig as SessionConfig, ZenohSection};
use stylos_session::SessionOverrides;
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::workflow::WorkflowState;
use tokio::sync::{mpsc, Notify, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zenoh::bytes::Encoding;
use zenoh::qos::CongestionControl;
use zenoh::query::{ConsolidationMode, QueryTarget};

use crate::config::StylosConfig;
use crate::Session;

const GIT_STATUS_TTL: Duration = Duration::from_secs(30);
const TASK_RETENTION: Duration = Duration::from_secs(30 * 60);
const MAX_WAIT_TIMEOUT_MS: u64 = 60_000;
const DISCOVERY_QUERY_TIMEOUT_MS: u64 = 1_500;

#[derive(Clone, Debug)]
pub enum StylosRuntimeState {
    Off,
    Active {
        mode: String,
        realm: String,
        instance: String,
    },
    Error(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct StylosAgentStatusSnapshot {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub session_id: String,
    pub workflow: WorkflowState,
    pub activity_status: String,
    pub activity_status_changed_at_ms: u64,
    pub project_dir: String,
    pub project_dir_is_git_repo: bool,
    pub git_remotes: Vec<String>,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub rate_limits: Option<ApiCallRateLimitReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StylosStatusSnapshot {
    pub startup_project_dir: String,
    pub agents: Vec<StylosAgentStatusSnapshot>,
}

#[derive(Clone, Debug)]
pub struct StylosCmdRequest {
    pub prompt: String,
}

#[derive(Clone, Debug)]
pub struct StylosRemotePromptRequest {
    pub prompt: String,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    #[allow(dead_code)]
    pub request_id: Option<String>,
}

type StylosSnapshotFuture = Pin<Box<dyn Future<Output = StylosStatusSnapshot> + Send>>;
type StylosSnapshotProvider = Arc<dyn Fn() -> StylosSnapshotFuture + Send + Sync>;

#[derive(Clone)]
pub struct StylosQueryContext {
    prompt_tx: mpsc::UnboundedSender<StylosRemotePromptRequest>,
    task_registry: TaskRegistry,
}

impl StylosQueryContext {
    pub fn submit_prompt(&self, request: StylosRemotePromptRequest) -> Result<(), String> {
        self.prompt_tx
            .send(request)
            .map_err(|_| "prompt queue unavailable".to_string())
    }

    pub fn task_registry(&self) -> &TaskRegistry {
        &self.task_registry
    }
}

#[derive(Clone)]
pub struct StylosToolBridge {
    instance: String,
    realm: String,
    session: Arc<zenoh::Session>,
    snapshot_provider: Arc<RwLock<Option<StylosSnapshotProvider>>>,
    query_context: StylosQueryContext,
    session_id: String,
}

impl StylosToolBridge {
    pub async fn invoke(&self, name: &str, args: serde_json::Value) -> anyhow::Result<String> {
        let reply = match name {
            "stylos_query_agents_alive" => serde_json::to_value(
                self.query_discovery::<serde_json::Value>(
                    &format!("stylos/{}/themion/query/agents/alive", self.realm),
                    None,
                )
                .await?,
            )?,
            "stylos_query_agents_free" => serde_json::to_value(
                self.query_discovery::<serde_json::Value>(
                    &format!("stylos/{}/themion/query/agents/free", self.realm),
                    None,
                )
                .await?,
            )?,
            "stylos_query_agents_git" => {
                let req = serde_json::from_value::<GitQueryRequest>(args)?;
                let payload = serde_cbor::to_vec(&req)?;
                serde_json::to_value(
                    self.query_discovery::<serde_json::Value>(
                        &format!("stylos/{}/themion/query/agents/git", self.realm),
                        Some(payload),
                    )
                    .await?,
                )?
            }
            "stylos_query_nodes" => serde_json::to_value(self.query_zenoh_nodes().await?)?,
            "stylos_query_status" => {
                let instance = args.get("instance").and_then(|v| v.as_str()).unwrap_or_default();
                if instance != self.instance {
                    json_error("not_found")
                } else {
                    let req = StatusFilterRequest {
                        agent_id: args.get("agent_id").and_then(|v| v.as_str()).map(str::to_string),
                        role: args.get("role").and_then(|v| v.as_str()).map(str::to_string),
                    };
                    serde_json::to_value(build_status_reply(&self.snapshot_provider, &self.instance, &self.session_id, Some(req)).await)?
                }
            }
            "stylos_request_talk" => {
                let req = TalkRequest {
                    agent_id: args.get("agent_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    message: args.get("message").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    request_id: args.get("request_id").and_then(|v| v.as_str()).map(str::to_string),
                };
                let instance = args.get("instance").and_then(|v| v.as_str()).unwrap_or_default();
                if instance != self.instance {
                    json_error("not_found")
                } else {
                    serde_json::to_value(handle_talk_query(&self.snapshot_provider, &self.query_context, Some(req)).await)?
                }
            }
            "stylos_request_task" => {
                let instance = args.get("instance").and_then(|v| v.as_str()).unwrap_or_default();
                if instance != self.instance {
                    json_error("not_found")
                } else {
                    let req = TaskRequestPayload {
                        task: args.get("task").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        preferred_agent_id: args.get("preferred_agent_id").and_then(|v| v.as_str()).map(str::to_string),
                        required_roles: args.get("required_roles").and_then(|v| serde_json::from_value(v.clone()).ok()),
                        require_git_repo: args.get("require_git_repo").and_then(|v| v.as_bool()),
                        request_id: args.get("request_id").and_then(|v| v.as_str()).map(str::to_string),
                    };
                    serde_json::to_value(handle_task_request_query(&self.snapshot_provider, &self.query_context, Some(req)).await)?
                }
            }
            "stylos_query_task_status" => {
                let instance = args.get("instance").and_then(|v| v.as_str()).unwrap_or_default();
                if instance != self.instance {
                    json_error("not_found")
                } else {
                    let req = TaskLookupRequest {
                        task_id: args.get("task_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    };
                    serde_json::to_value(handle_task_status_query(&self.query_context, Some(req)).await)?
                }
            }
            "stylos_query_task_result" => {
                let instance = args.get("instance").and_then(|v| v.as_str()).unwrap_or_default();
                if instance != self.instance {
                    json_error("not_found")
                } else {
                    let req = TaskResultRequest {
                        task_id: args.get("task_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        wait_timeout_ms: args.get("wait_timeout_ms").and_then(|v| v.as_u64()),
                    };
                    serde_json::to_value(handle_task_result_query(&self.query_context, Some(req)).await)?
                }
            }
            _ => anyhow::bail!("unknown stylos tool: {name}"),
        };
        Ok(reply.to_string())
    }
}


impl StylosToolBridge {
    async fn query_zenoh_nodes(&self) -> anyhow::Result<ZenohNodesReply> {
        let info = self.session.info();
        let self_zid = info.zid().await.to_string();
        let mut peer_zids: Vec<String> = info.peers_zid().await.map(|zid| zid.to_string()).collect();
        let mut router_zids: Vec<String> = info.routers_zid().await.map(|zid| zid.to_string()).collect();
        peer_zids.sort();
        peer_zids.dedup();
        router_zids.sort();
        router_zids.dedup();
        Ok(ZenohNodesReply {
            self_zid,
            peer_zids,
            router_zids,
        })
    }
}

impl StylosToolBridge {
    async fn query_discovery<T>(&self, key: &str, payload: Option<Vec<u8>>) -> anyhow::Result<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut builder = self
            .session
            .get(key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS));
        if let Some(payload) = payload {
            builder = builder.payload(payload).encoding(Encoding::APPLICATION_CBOR);
        }
        let replies = builder.await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut out = Vec::new();
        let mut stream = replies.into_stream();
        loop {
            match tokio::time::timeout(Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS), stream.next()).await {
                Ok(Some(reply)) => {
                    let decoded = match reply.into_result() {
                        Ok(sample) => serde_cbor::from_slice::<T>(sample.payload().to_bytes().as_ref())?,
                        Err(err) => {
                            let reason = err.to_string();
                            return Err(anyhow::anyhow!(reason));
                        }
                    };
                    out.push(decoded);
                }
                Ok(None) | Err(_) => break,
            }
        }
        Ok(out)
    }
}

fn json_error(reason: &str) -> serde_json::Value {
    serde_json::json!({"error": reason})
}

pub struct StylosHandle {
    state: StylosRuntimeState,
    session_id: Option<String>,
    session: Option<Arc<zenoh::Session>>,
    status_task: Option<JoinHandle<()>>,
    queryable_task: Option<JoinHandle<()>>,
    cmd_task: Option<JoinHandle<()>>,
    cmd_rx: Option<mpsc::UnboundedReceiver<StylosCmdRequest>>,
    prompt_rx: Option<mpsc::UnboundedReceiver<StylosRemotePromptRequest>>,
    snapshot_provider: Arc<RwLock<Option<StylosSnapshotProvider>>>,
    query_context: StylosQueryContext,
}

impl StylosHandle {
    pub fn off() -> Self {
        let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
        Self {
            state: StylosRuntimeState::Off,
            session_id: None,
            session: None,
            status_task: None,
            queryable_task: None,
            cmd_task: None,
            cmd_rx: None,
            prompt_rx: Some(prompt_rx),
            snapshot_provider: Arc::new(RwLock::new(None)),
            query_context: StylosQueryContext {
                prompt_tx,
                task_registry: TaskRegistry::new(),
            },
        }
    }

    pub fn state(&self) -> &StylosRuntimeState {
        &self.state
    }

    pub fn take_cmd_rx(&mut self) -> Option<mpsc::UnboundedReceiver<StylosCmdRequest>> {
        self.cmd_rx.take()
    }

    pub fn take_prompt_rx(&mut self) -> Option<mpsc::UnboundedReceiver<StylosRemotePromptRequest>> {
        self.prompt_rx.take()
    }

    pub fn query_context(&self) -> StylosQueryContext {
        self.query_context.clone()
    }

    pub async fn set_snapshot_provider(&self, provider: StylosSnapshotProvider) {
        *self.snapshot_provider.write().await = Some(provider);
    }

    pub fn session_id_string(&self) -> String {
        self.session_id.clone().unwrap_or_default()
    }

    pub async fn shutdown(self) {
        if let Some(task) = self.status_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.queryable_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.cmd_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(session) = self.session {
            let _ = session.close().await;
        }
    }
}

#[derive(Serialize)]
struct ThemionInfo {
    version: String,
    instance: String,
    realm: String,
    mode: String,
    profile: String,
    model: String,
}

#[derive(Serialize)]
struct ThemionStatusPayload {
    version: String,
    instance: String,
    realm: String,
    mode: String,
    startup_project_dir: String,
    agents: Vec<StylosAgentStatusSnapshot>,
}

#[derive(Debug, Deserialize)]
struct ThemionCmdPayload {
    r#type: String,
    prompt: String,
}

#[derive(Clone, Debug)]
pub struct GitStatusCache {
    project_dir: PathBuf,
    state: Arc<std::sync::Mutex<CachedGitStatus>>,
}

#[derive(Clone, Debug)]
struct CachedGitStatus {
    last_refresh: Instant,
    value: GitProjectStatus,
}

impl GitStatusCache {
    pub fn new(project_dir: PathBuf) -> Self {
        let value = inspect_git_project(&project_dir);
        Self {
            project_dir,
            state: Arc::new(std::sync::Mutex::new(CachedGitStatus {
                last_refresh: Instant::now(),
                value,
            })),
        }
    }

    pub fn snapshot(&self) -> GitProjectStatus {
        let mut state = self.state.lock().expect("git status cache lock");
        if state.last_refresh.elapsed() >= GIT_STATUS_TTL {
            state.value = inspect_git_project(&self.project_dir);
            state.last_refresh = Instant::now();
        }
        state.value.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitProjectStatus {
    pub is_repo: bool,
    pub remotes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StylosQueryableAgentSnapshot {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub session_id: String,
    pub activity_status: String,
    pub activity_status_changed_at_ms: u64,
    pub project_dir: String,
    pub project_dir_is_git_repo: bool,
    pub git_remotes: Vec<String>,
    pub git_repo_keys: Vec<String>,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub workflow: WorkflowState,
    pub rate_limits: Option<ApiCallRateLimitReport>,
}

#[derive(Clone, Debug, Serialize)]
struct DiscoveryReply {
    instance: String,
    session_id: String,
    agents: Vec<StylosQueryableAgentSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
struct ZenohNodesReply {
    self_zid: String,
    peer_zids: Vec<String>,
    router_zids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StatusFilterRequest {
    agent_id: Option<String>,
    role: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct StatusReply {
    found: bool,
    instance: String,
    session_id: String,
    startup_project_dir: String,
    agents: Vec<StylosQueryableAgentSnapshot>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GitQueryRequest {
    remote: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TalkRequest {
    agent_id: String,
    message: String,
    request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TalkReply {
    accepted: bool,
    agent_id: String,
    request_id: Option<String>,
    correlation_id: Option<String>,
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskRequestPayload {
    task: String,
    preferred_agent_id: Option<String>,
    required_roles: Option<Vec<String>>,
    require_git_repo: Option<bool>,
    request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskRequestReply {
    accepted: bool,
    agent_id: Option<String>,
    request_id: Option<String>,
    task_id: Option<String>,
    note: Option<String>,
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskLookupRequest {
    task_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskResultRequest {
    task_id: String,
    wait_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct TaskRegistry {
    inner: Arc<RwLock<HashMap<String, TaskEntry>>>,
    notify: Arc<Notify>,
}

#[derive(Clone, Debug)]
struct TaskEntry {
    task_id: String,
    state: String,
    agent_id: String,
    result: Option<String>,
    reason: Option<String>,
    updated_at: Instant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskLookupReply {
    found: bool,
    task_id: String,
    state: Option<String>,
    agent_id: Option<String>,
    result: Option<String>,
    reason: Option<String>,
    timed_out: Option<bool>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn insert_queued(&self, task_id: String, agent_id: String) {
        self.inner.write().await.insert(
            task_id.clone(),
            TaskEntry {
                task_id,
                state: "queued".to_string(),
                agent_id,
                result: None,
                reason: None,
                updated_at: Instant::now(),
            },
        );
        self.notify.notify_waiters();
    }

    pub async fn set_running(&self, task_id: &str) {
        if let Some(entry) = self.inner.write().await.get_mut(task_id) {
            entry.state = "running".to_string();
            entry.updated_at = Instant::now();
        }
        self.notify.notify_waiters();
    }

    pub async fn set_failed(&self, task_id: &str, reason: String) {
        self.set_completed(task_id, None, Some(reason)).await;
    }

    async fn get(&self, task_id: &str) -> Option<TaskEntry> {
        self.expire_old().await;
        self.inner.read().await.get(task_id).cloned()
    }

    pub async fn set_completed(&self, task_id: &str, result: Option<String>, reason: Option<String>) {
        if let Some(entry) = self.inner.write().await.get_mut(task_id) {
            entry.state = if reason.is_some() { "failed".to_string() } else { "completed".to_string() };
            entry.result = result;
            entry.reason = reason;
            entry.updated_at = Instant::now();
        }
        self.notify.notify_waiters();
    }

    async fn expire_old(&self) {
        self.inner
            .write()
            .await
            .retain(|_, entry| entry.updated_at.elapsed() < TASK_RETENTION);
    }

    async fn wait_for_terminal(&self, task_id: &str, timeout_ms: u64) -> Option<TaskEntry> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(entry) = self.get(task_id).await {
                if matches!(entry.state.as_str(), "completed" | "failed" | "rejected" | "expired") {
                    return Some(entry);
                }
            } else {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return self.get(task_id).await;
            }
            let remaining = deadline.saturating_duration_since(now);
            let notified = self.notify.notified();
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return self.get(task_id).await;
            }
        }
    }
}

pub async fn start(
    settings: &StylosConfig,
    session: &Session,
    project_dir: &PathBuf,
) -> StylosHandle {
    if !settings.enabled() {
        return StylosHandle::off();
    }

    match start_inner(settings, session, project_dir).await {
        Ok(handle) => handle,
        Err(err) => {
            let mut handle = StylosHandle::off();
            handle.state = StylosRuntimeState::Error(err);
            handle
        }
    }
}

async fn start_inner(
    settings: &StylosConfig,
    session: &Session,
    project_dir: &PathBuf,
) -> Result<StylosHandle, String> {
    let hostname = derive_hostname().unwrap_or_else(|| "themion".to_string());
    let git_status = inspect_git_project(project_dir);
    let process_id = std::process::id();
    let identity_instance = hostname.clone();
    let key_instance = format!("{hostname}/{process_id}");
    let realm = settings.realm();
    let mode = settings.mode();

    let cfg = SessionConfig {
        stylos: IdentitySection {
            realm: realm.clone(),
            role: "themion".to_string(),
            instance: identity_instance.clone(),
        },
        zenoh: ZenohSection {
            mode: mode.clone(),
            connect: Endpoints {
                endpoints: settings.connect.clone(),
            },
            listen: Endpoints::default(),
            scouting: None,
        },
    };

    let overrides = SessionOverrides {
        connect: if settings.connect.is_empty() {
            None
        } else {
            Some(settings.connect.clone())
        },
    };

    let session_handle = Arc::new(
        stylos_session::open_session(&cfg, &overrides)
            .await
            .map_err(|e| e.to_string())?,
    );

    let ct = CancellationToken::new();
    let snapshot_provider = Arc::new(RwLock::new(None::<StylosSnapshotProvider>));
    let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
    let task_registry = TaskRegistry::new();
    let query_context = StylosQueryContext {
        prompt_tx,
        task_registry: task_registry.clone(),
    };

    let status_ct = ct.clone();
    let status_session = session_handle.clone();
    let status_key = format!("stylos/{}/themion/{}/status", realm, key_instance);
    let status_snapshot_provider = snapshot_provider.clone();
    let initial_project_dir = project_dir.display().to_string();
    let initial_project_dir_is_git_repo = git_status.is_repo;
    let initial_git_remotes = git_status.remotes.clone();
    let status_profile = session.active_profile.clone();
    let status_provider = session.provider.clone();
    let status_model = session.model.clone();
    let status_session_id = session.id.to_string();
    let status_mode = mode.clone();
    let status_realm = realm.clone();
    let status_instance = key_instance.clone();
    let status_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = status_ct.cancelled() => break,
                _ = interval.tick() => {
                    let provider = status_snapshot_provider.read().await.clone();
                    let snapshot = match provider {
                        Some(provider) => provider().await,
                        None => StylosStatusSnapshot {
                            startup_project_dir: initial_project_dir.clone(),
                            agents: vec![StylosAgentStatusSnapshot {
                                agent_id: "main".to_string(),
                                label: "main".to_string(),
                                roles: vec!["main".to_string(), "interactive".to_string()],
                                session_id: status_session_id.clone(),
                                workflow: WorkflowState::default(),
                                activity_status: "idle".to_string(),
                                activity_status_changed_at_ms: unix_epoch_now_ms(),
                                project_dir: initial_project_dir.clone(),
                                project_dir_is_git_repo: initial_project_dir_is_git_repo,
                                git_remotes: initial_git_remotes.clone(),
                                provider: status_provider.clone(),
                                model: status_model.clone(),
                                active_profile: status_profile.clone(),
                                rate_limits: None,
                            }],
                        },
                    };
                    let payload = ThemionStatusPayload {
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        instance: status_instance.clone(),
                        realm: status_realm.clone(),
                        mode: status_mode.clone(),
                        startup_project_dir: snapshot.startup_project_dir,
                        agents: snapshot.agents,
                    };
                    if let Ok(bytes) = serde_cbor::to_vec(&payload) {
                        let _ = status_session
                            .put(&status_key, bytes)
                            .encoding(Encoding::APPLICATION_CBOR)
                            .congestion_control(CongestionControl::Drop)
                            .await;
                    }
                }
            }
        }
    });

    let q_ct = ct.clone();
    let q_session = session_handle.clone();
    let q_info_key = format!("stylos/{}/themion/{}/info", realm, key_instance);
    let q_alive_key = format!("stylos/{}/themion/query/agents/alive", realm);
    let q_free_key = format!("stylos/{}/themion/query/agents/free", realm);
    let q_git_key = format!("stylos/{}/themion/query/agents/git", realm);
    let q_status_key = format!("stylos/{}/themion/{}/query/status", realm, key_instance);
    let q_talk_key = format!("stylos/{}/themion/{}/query/talk", realm, key_instance);
    let q_task_request_key = format!("stylos/{}/themion/{}/query/tasks/request", realm, key_instance);
    let q_task_status_key = format!("stylos/{}/themion/{}/query/tasks/status", realm, key_instance);
    let q_task_result_key = format!("stylos/{}/themion/{}/query/tasks/result", realm, key_instance);
    let info = ThemionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        instance: key_instance.clone(),
        realm: realm.clone(),
        mode: mode.clone(),
        profile: session.active_profile.clone(),
        model: session.model.clone(),
    };
    let query_snapshot_provider = snapshot_provider.clone();
    let query_context_for_task = query_context.clone();
    let query_instance = key_instance.clone();
    let query_session_id = session.id.to_string();
    let queryable_task = tokio::spawn(async move {
        let info_queryable = match q_session.declare_queryable(&q_info_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let alive_queryable = match q_session.declare_queryable(&q_alive_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let free_queryable = match q_session.declare_queryable(&q_free_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let git_queryable = match q_session.declare_queryable(&q_git_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let status_queryable = match q_session.declare_queryable(&q_status_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let talk_queryable = match q_session.declare_queryable(&q_talk_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let task_request_queryable = match q_session.declare_queryable(&q_task_request_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let task_status_queryable = match q_session.declare_queryable(&q_task_status_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let task_result_queryable = match q_session.declare_queryable(&q_task_result_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let info_payload = serde_json::to_vec(&info).unwrap_or_default();

        loop {
            tokio::select! {
                _ = q_ct.cancelled() => break,
                res = info_queryable.recv_async() => match res {
                    Ok(query) => {
                        let _ = query.reply(q_info_key.clone(), info_payload.clone())
                            .encoding(Encoding::APPLICATION_JSON)
                            .await;
                    }
                    Err(_) => break,
                },
                res = alive_queryable.recv_async() => match res {
                    Ok(query) => {
                        if let Some(reply) = build_discovery_reply(&query_snapshot_provider, &query_instance, &query_session_id, DiscoveryMode::Alive).await {
                            let _ = reply_cbor(query, q_alive_key.clone(), &reply).await;
                        }
                    }
                    Err(_) => break,
                },
                res = free_queryable.recv_async() => match res {
                    Ok(query) => {
                        if let Some(reply) = build_discovery_reply(&query_snapshot_provider, &query_instance, &query_session_id, DiscoveryMode::Free).await {
                            let _ = reply_cbor(query, q_free_key.clone(), &reply).await;
                        }
                    }
                    Err(_) => break,
                },
                res = git_queryable.recv_async() => match res {
                    Ok(query) => {
                        let req = parse_cbor_payload::<GitQueryRequest>(&query);
                        if let Some(reply) = build_git_reply(&query_snapshot_provider, &query_instance, &query_session_id, req).await {
                            let _ = reply_cbor(query, q_git_key.clone(), &reply).await;
                        }
                    }
                    Err(_) => break,
                },
                res = status_queryable.recv_async() => match res {
                    Ok(query) => {
                        let req = parse_cbor_payload::<StatusFilterRequest>(&query);
                        let reply = build_status_reply(&query_snapshot_provider, &query_instance, &query_session_id, req).await;
                        let _ = reply_cbor(query, q_status_key.clone(), &reply).await;
                    }
                    Err(_) => break,
                },
                res = talk_queryable.recv_async() => match res {
                    Ok(query) => {
                        let reply = handle_talk_query(&query_snapshot_provider, &query_context_for_task, parse_cbor_payload::<TalkRequest>(&query)).await;
                        let _ = reply_cbor(query, q_talk_key.clone(), &reply).await;
                    }
                    Err(_) => break,
                },
                res = task_request_queryable.recv_async() => match res {
                    Ok(query) => {
                        let reply = handle_task_request_query(&query_snapshot_provider, &query_context_for_task, parse_cbor_payload::<TaskRequestPayload>(&query)).await;
                        let _ = reply_cbor(query, q_task_request_key.clone(), &reply).await;
                    }
                    Err(_) => break,
                },
                res = task_status_queryable.recv_async() => match res {
                    Ok(query) => {
                        let reply = handle_task_status_query(&query_context_for_task, parse_cbor_payload::<TaskLookupRequest>(&query)).await;
                        let _ = reply_cbor(query, q_task_status_key.clone(), &reply).await;
                    }
                    Err(_) => break,
                },
                res = task_result_queryable.recv_async() => match res {
                    Ok(query) => {
                        let reply = handle_task_result_query(&query_context_for_task, parse_cbor_payload::<TaskResultRequest>(&query)).await;
                        let _ = reply_cbor(query, q_task_result_key.clone(), &reply).await;
                    }
                    Err(_) => break,
                },
            }
        }
    });

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let cmd_ct = ct.clone();
    let cmd_session = session_handle.clone();
    let cmd_key = format!("stylos/{}/themion/{}/cmd", realm, key_instance);
    let cmd_task = tokio::spawn(async move {
        let subscriber = match cmd_session.declare_subscriber(&cmd_key).await {
            Ok(sub) => sub,
            Err(_) => return,
        };
        loop {
            tokio::select! {
                _ = cmd_ct.cancelled() => break,
                res = subscriber.recv_async() => match res {
                    Ok(sample) => {
                        let Ok(payload) = serde_cbor::from_slice::<ThemionCmdPayload>(sample.payload().to_bytes().as_ref()) else {
                            continue;
                        };
                        if payload.r#type != "text_prompt" {
                            continue;
                        }
                        let prompt = payload.prompt.trim().to_string();
                        if prompt.is_empty() {
                            continue;
                        }
                        let _ = cmd_tx.send(StylosCmdRequest { prompt });
                    }
                    Err(_) => break,
                }
            }
        }
    });

    Ok(StylosHandle {
        state: StylosRuntimeState::Active {
            mode,
            realm,
            instance: key_instance,
        },
        session_id: Some(session.id.to_string()),
        session: Some(session_handle),
        status_task: Some(status_task),
        queryable_task: Some(queryable_task),
        cmd_task: Some(cmd_task),
        cmd_rx: Some(cmd_rx),
        prompt_rx: Some(prompt_rx),
        snapshot_provider,
        query_context,
    })
}

#[derive(Clone, Copy)]
enum DiscoveryMode {
    Alive,
    Free,
}

async fn build_discovery_reply(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
    instance: &str,
    session_id: &str,
    mode: DiscoveryMode,
) -> Option<DiscoveryReply> {
    let snapshot = current_snapshot(snapshot_provider).await?;
    let mut agents = build_queryable_agents(snapshot.agents);
    if matches!(mode, DiscoveryMode::Free) {
        agents.retain(|agent| matches!(agent.activity_status.as_str(), "idle" | "nap"));
    }
    Some(DiscoveryReply {
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        agents,
    })
}

async fn build_git_reply(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
    instance: &str,
    session_id: &str,
    req: Option<GitQueryRequest>,
) -> Option<DiscoveryReply> {
    let snapshot = current_snapshot(snapshot_provider).await?;
    let mut agents = build_queryable_agents(snapshot.agents);
    let requested = req.and_then(|r| r.remote);
    let requested_key = requested.as_deref().and_then(normalize_git_remote);
    agents.retain(|agent| {
        if !agent.project_dir_is_git_repo {
            return false;
        }
        match (requested.as_ref(), requested_key.as_ref()) {
            (None, _) => true,
            (Some(raw), Some(key)) => {
                agent.git_repo_keys.iter().any(|candidate| candidate == key)
                    || agent.git_remotes.iter().any(|remote| remote == raw)
            }
            (Some(raw), None) => agent.git_remotes.iter().any(|remote| remote == raw),
        }
    });
    Some(DiscoveryReply {
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        agents,
    })
}

async fn build_status_reply(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
    instance: &str,
    session_id: &str,
    req: Option<StatusFilterRequest>,
) -> StatusReply {
    let Some(snapshot) = current_snapshot(snapshot_provider).await else {
        return StatusReply {
            found: false,
            instance: instance.to_string(),
            session_id: session_id.to_string(),
            startup_project_dir: String::new(),
            agents: Vec::new(),
            error: Some("snapshot_unavailable".to_string()),
        };
    };
    let mut agents = build_queryable_agents(snapshot.agents);
    let mut found = true;
    if let Some(req) = req {
        if req.agent_id.is_some() && req.role.is_some() {
            return StatusReply {
                found: false,
                instance: instance.to_string(),
                session_id: session_id.to_string(),
                startup_project_dir: snapshot.startup_project_dir,
                agents: Vec::new(),
                error: Some("invalid_request".to_string()),
            };
        }
        if let Some(agent_id) = req.agent_id {
            agents.retain(|agent| agent.agent_id == agent_id);
            if agents.is_empty() {
                found = false;
            }
        }
        if let Some(role) = req.role {
            agents.retain(|agent| agent.roles.iter().any(|r| r == &role));
        }
    }
    StatusReply {
        found,
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        startup_project_dir: snapshot.startup_project_dir,
        agents,
        error: if found { None } else { Some("not_found".to_string()) },
    }
}

async fn handle_talk_query(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
    query_context: &StylosQueryContext,
    req: Option<TalkRequest>,
) -> TalkReply {
    let Some(req) = req else {
        return TalkReply {
            accepted: false,
            agent_id: String::new(),
            request_id: None,
            correlation_id: None,
            reason: Some("invalid_request".to_string()),
        };
    };
    let Some(snapshot) = current_snapshot(snapshot_provider).await else {
        return TalkReply {
            accepted: false,
            agent_id: req.agent_id,
            request_id: req.request_id,
            correlation_id: None,
            reason: Some("snapshot_unavailable".to_string()),
        };
    };
    let agent = snapshot.agents.into_iter().find(|a| a.agent_id == req.agent_id);
    let Some(agent) = agent else {
        return TalkReply {
            accepted: false,
            agent_id: req.agent_id,
            request_id: req.request_id,
            correlation_id: None,
            reason: Some("not_found".to_string()),
        };
    };
    if !matches!(agent.activity_status.as_str(), "idle" | "nap") {
        return TalkReply {
            accepted: false,
            agent_id: agent.agent_id,
            request_id: req.request_id,
            correlation_id: None,
            reason: Some("agent_busy".to_string()),
        };
    }
    let correlation_id = format!("talk-{}", Uuid::new_v4());
    let result = query_context.submit_prompt(StylosRemotePromptRequest {
        prompt: req.message,
        agent_id: Some(agent.agent_id.clone()),
        task_id: None,
        request_id: req.request_id.clone(),
    });
    TalkReply {
        accepted: result.is_ok(),
        agent_id: agent.agent_id,
        request_id: req.request_id,
        correlation_id: if result.is_ok() { Some(correlation_id) } else { None },
        reason: result.err(),
    }
}

async fn handle_task_request_query(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
    query_context: &StylosQueryContext,
    req: Option<TaskRequestPayload>,
) -> TaskRequestReply {
    let Some(req) = req else {
        return TaskRequestReply {
            accepted: false,
            agent_id: None,
            request_id: None,
            task_id: None,
            note: None,
            reason: Some("invalid_request".to_string()),
        };
    };
    let Some(snapshot) = current_snapshot(snapshot_provider).await else {
        return TaskRequestReply {
            accepted: false,
            agent_id: None,
            request_id: req.request_id,
            task_id: None,
            note: None,
            reason: Some("snapshot_unavailable".to_string()),
        };
    };

    let mut candidates: Vec<_> = snapshot
        .agents
        .into_iter()
        .filter(|agent| matches!(agent.activity_status.as_str(), "idle" | "nap"))
        .collect();

    if let Some(preferred) = req.preferred_agent_id.as_ref() {
        candidates.retain(|agent| &agent.agent_id == preferred);
    }
    if let Some(required_roles) = req.required_roles.as_ref() {
        candidates.retain(|agent| required_roles.iter().all(|role| agent.roles.iter().any(|r| r == role)));
    }
    if req.require_git_repo.unwrap_or(false) {
        candidates.retain(|agent| agent.project_dir_is_git_repo);
    }
    candidates.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

    let Some(agent) = candidates.into_iter().next() else {
        return TaskRequestReply {
            accepted: false,
            agent_id: None,
            request_id: req.request_id,
            task_id: None,
            note: None,
            reason: Some("no_available_agent".to_string()),
        };
    };

    let task_id = format!("task-{}", Uuid::new_v4());
    query_context
        .task_registry()
        .insert_queued(task_id.clone(), agent.agent_id.clone())
        .await;
    let submit_result = query_context.submit_prompt(StylosRemotePromptRequest {
        prompt: req.task,
        agent_id: Some(agent.agent_id.clone()),
        task_id: Some(task_id.clone()),
        request_id: req.request_id.clone(),
    });
    if let Err(reason) = submit_result {
        query_context
            .task_registry()
            .set_completed(&task_id, None, Some(reason.clone()))
            .await;
        return TaskRequestReply {
            accepted: false,
            agent_id: Some(agent.agent_id),
            request_id: req.request_id,
            task_id: Some(task_id),
            note: None,
            reason: Some(reason),
        };
    }

    TaskRequestReply {
        accepted: true,
        agent_id: Some(agent.agent_id),
        request_id: req.request_id,
        task_id: Some(task_id),
        note: Some("queued for local delivery".to_string()),
        reason: None,
    }
}

async fn handle_task_status_query(
    query_context: &StylosQueryContext,
    req: Option<TaskLookupRequest>,
) -> TaskLookupReply {
    let Some(req) = req else {
        return TaskLookupReply {
            found: false,
            task_id: String::new(),
            state: None,
            agent_id: None,
            result: None,
            reason: Some("invalid_request".to_string()),
            timed_out: None,
        };
    };
    match query_context.task_registry().get(&req.task_id).await {
        Some(entry) => TaskLookupReply {
            found: true,
            task_id: entry.task_id,
            state: Some(entry.state),
            agent_id: Some(entry.agent_id),
            result: entry.result,
            reason: entry.reason,
            timed_out: None,
        },
        None => TaskLookupReply {
            found: false,
            task_id: req.task_id,
            state: None,
            agent_id: None,
            result: None,
            reason: Some("not_found".to_string()),
            timed_out: None,
        },
    }
}

async fn handle_task_result_query(
    query_context: &StylosQueryContext,
    req: Option<TaskResultRequest>,
) -> TaskLookupReply {
    let Some(req) = req else {
        return TaskLookupReply {
            found: false,
            task_id: String::new(),
            state: None,
            agent_id: None,
            result: None,
            reason: Some("invalid_request".to_string()),
            timed_out: None,
        };
    };
    let wait_timeout_ms = req.wait_timeout_ms.unwrap_or(0).min(MAX_WAIT_TIMEOUT_MS);
    let entry = if wait_timeout_ms == 0 {
        query_context.task_registry().get(&req.task_id).await
    } else {
        query_context
            .task_registry()
            .wait_for_terminal(&req.task_id, wait_timeout_ms)
            .await
    };
    match entry {
        Some(entry) => {
            let timed_out = wait_timeout_ms > 0
                && !matches!(entry.state.as_str(), "completed" | "failed" | "rejected" | "expired");
            TaskLookupReply {
                found: true,
                task_id: entry.task_id,
                state: Some(entry.state),
                agent_id: Some(entry.agent_id),
                result: entry.result,
                reason: entry.reason,
                timed_out: Some(timed_out),
            }
        }
        None => TaskLookupReply {
            found: false,
            task_id: req.task_id,
            state: None,
            agent_id: None,
            result: None,
            reason: Some("not_found".to_string()),
            timed_out: None,
        },
    }
}

async fn current_snapshot(
    snapshot_provider: &Arc<RwLock<Option<StylosSnapshotProvider>>>,
) -> Option<StylosStatusSnapshot> {
    let provider = snapshot_provider.read().await.clone()?;
    Some(provider().await)
}

fn build_queryable_agents(agents: Vec<StylosAgentStatusSnapshot>) -> Vec<StylosQueryableAgentSnapshot> {
    agents
        .into_iter()
        .map(|agent| {
            let git_repo_keys = derive_git_repo_keys(&agent.git_remotes);
            StylosQueryableAgentSnapshot {
                agent_id: agent.agent_id,
                label: agent.label,
                roles: agent.roles,
                session_id: agent.session_id,
                activity_status: agent.activity_status,
                activity_status_changed_at_ms: agent.activity_status_changed_at_ms,
                project_dir: agent.project_dir,
                project_dir_is_git_repo: agent.project_dir_is_git_repo,
                git_remotes: agent.git_remotes,
                git_repo_keys,
                provider: agent.provider,
                model: agent.model,
                active_profile: agent.active_profile,
                workflow: agent.workflow,
                rate_limits: agent.rate_limits,
            }
        })
        .collect()
}

fn derive_git_repo_keys(remotes: &[String]) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for remote in remotes {
        if let Some(key) = normalize_git_remote(remote) {
            keys.insert(key);
        }
    }
    keys.into_iter().collect()
}

fn normalize_git_remote(remote: &str) -> Option<String> {
    let trimmed = remote.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((user_host, path)) = trimmed.split_once(':') {
        if let Some(host) = user_host.strip_prefix("git@") {
            return normalize_known_host_path(host, path);
        }
    }

    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("ssh://git@"))
        .or_else(|| trimmed.strip_prefix("ssh://"));

    let Some(rest) = without_scheme else {
        return None;
    };
    let (host, path) = rest.split_once('/')?;
    normalize_known_host_path(host, path)
}

fn normalize_known_host_path(host: &str, path: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    if !matches!(host.as_str(), "github.com" | "gitlab.com" | "bitbucket.org") {
        return None;
    }
    let path = path.trim().trim_matches('/').trim_end_matches(".git");
    if path.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host, path))
}

fn parse_cbor_payload<T: for<'de> Deserialize<'de>>(query: &zenoh::query::Query) -> Option<T> {
    let payload = query.payload()?;
    let bytes = payload.to_bytes();
    if bytes.is_empty() {
        return None;
    }
    serde_cbor::from_slice(bytes.as_ref()).ok()
}

async fn reply_cbor<T: Serialize>(query: zenoh::query::Query, key: String, payload: &T) -> Result<(), zenoh::Error> {
    let bytes = serde_cbor::to_vec(payload).unwrap_or_default();
    query.reply(key, bytes).encoding(Encoding::APPLICATION_CBOR).await
}

fn inspect_git_project(project_dir: &Path) -> GitProjectStatus {
    let inside = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let is_repo = match inside {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim() == "true"
        }
        _ => false,
    };

    let remotes = if is_repo {
        collect_git_remote_urls(project_dir)
    } else {
        Vec::new()
    };

    GitProjectStatus { is_repo, remotes }
}

fn collect_git_remote_urls(project_dir: &Path) -> Vec<String> {
    let output = match std::process::Command::new("git")
        .arg("remote")
        .arg("-v")
        .current_dir(project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    let mut urls = BTreeSet::<String>::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_whitespace();
        let _name = parts.next();
        let Some(url) = parts.next() else {
            continue;
        };
        urls.insert(url.to_string());
    }

    urls.into_iter().collect()
}

fn derive_hostname() -> Option<String> {
    let hostname = hostname::get().ok()?.to_string_lossy().to_lowercase();
    let mapped: String = hostname
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches('-').to_string();
    let capped: String = trimmed.chars().take(32).collect();
    if capped.is_empty() || !is_valid_segment(&capped) {
        None
    } else {
        Some(capped)
    }
}

fn is_valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn unix_epoch_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_git_remote_forms() {
        assert_eq!(
            normalize_git_remote("git@github.com:example/themion.git").as_deref(),
            Some("github.com/example/themion")
        );
        assert_eq!(
            normalize_git_remote("https://github.com/example/themion").as_deref(),
            Some("github.com/example/themion")
        );
        assert_eq!(
            normalize_git_remote("ssh://git@gitlab.com/group/proj.git").as_deref(),
            Some("gitlab.com/group/proj")
        );
        assert_eq!(
            normalize_git_remote("git@bitbucket.org:team/repo.git").as_deref(),
            Some("bitbucket.org/team/repo")
        );
    }

    #[test]
    fn unsupported_hosts_do_not_normalize() {
        assert_eq!(normalize_git_remote("git@example.com:repo.git"), None);
    }

    #[tokio::test]
    async fn task_registry_wait_returns_current_state_after_timeout() {
        let registry = TaskRegistry::new();
        registry
            .insert_queued("task-1".to_string(), "agent-1".to_string())
            .await;
        let entry = registry.wait_for_terminal("task-1", 10).await.unwrap();
        assert_eq!(entry.state, "queued");
    }
}

pub fn tool_bridge(handle: &StylosHandle) -> Option<StylosToolBridge> {
    match handle.state() {
        StylosRuntimeState::Active { realm, instance, .. } => Some(StylosToolBridge {
            instance: instance.clone(),
            realm: realm.clone(),
            session: handle.session.as_ref()?.clone(),
            snapshot_provider: handle.snapshot_provider.clone(),
            query_context: handle.query_context(),
            session_id: handle.session_id_string(),
        }),
        _ => None,
    }
}

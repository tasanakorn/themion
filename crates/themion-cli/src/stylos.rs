#![cfg(feature = "stylos")]

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use themion_core::db::{CreateNoteArgs, NoteColumn, NoteKind};

use serde::{Deserialize, Serialize};
use stylos::{
    Endpoints, IdentitySection, SessionOverrides, StylosConfig as SessionConfig, ZenohSection,
};
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::workflow::WorkflowState;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app_runtime::{SharedStylosStatusHub, StylosSnapshotProvider};
use crate::runtime_domains::DomainHandle;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zenoh::bytes::Encoding;
use zenoh::qos::CongestionControl;
use zenoh::query::{ConsolidationMode, QueryTarget};

use crate::config::StylosConfig;
use crate::Session;

const GIT_STATUS_TTL: Duration = Duration::from_secs(30);

const DISCOVERY_QUERY_TIMEOUT_MS: u64 = 1_500;
pub(crate) const MAX_INBOX_MESSAGES_PER_AGENT: usize = 16;
pub(crate) const INBOX_MESSAGE_TTL_MS: u64 = 600_000;

const PRIMARY_AGENT_ID: &str = "master";
const PRIMARY_AGENT_ID_COMPAT_ALIAS: &str = "main";
const PRIMARY_ROLE: &str = "master";
const PRIMARY_ROLE_COMPAT_ALIAS: &str = "main";

fn normalize_primary_agent_id(value: &str) -> &str {
    if value == PRIMARY_AGENT_ID_COMPAT_ALIAS {
        PRIMARY_AGENT_ID
    } else {
        value
    }
}

fn normalize_primary_role(value: &str) -> &str {
    if value == PRIMARY_ROLE_COMPAT_ALIAS {
        PRIMARY_ROLE
    } else {
        value
    }
}

#[derive(Clone, Debug)]
pub struct SenderSideTransportEvent {
    pub agent_id: Option<String>,
    pub text: String,
}

pub fn sender_side_transport_event_from_tool_detail(
    detail: &str,
    local_instance: &str,
    use_stylos_note_delivery: bool,
) -> Option<SenderSideTransportEvent> {
    if let Some(target) = extract_stylos_message_target_from_detail(detail) {
        return Some(SenderSideTransportEvent {
            agent_id: None,
            text: format!("Stylos message to={} from={}", target, local_instance),
        });
    }

    if detail.starts_with("board_create_note") {
        let mode = if use_stylos_note_delivery {
            "send request via stylos"
        } else {
            "create local"
        };
        return Some(SenderSideTransportEvent {
            agent_id: None,
            text: format!(
                "board_create_note {} from={} detail={}",
                mode, local_instance, detail
            ),
        });
    }

    None
}

fn extract_stylos_message_target_from_detail(detail: &str) -> Option<&str> {
    let prefix = "stylos_send_message ";
    let rest = detail.strip_prefix(prefix)?;
    for field in rest.split_whitespace() {
        if let Some(value) = field.strip_prefix("instance=") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

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
    pub app_version: String,
    pub app_version_hash: String,
    pub app_version_dirty: bool,
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

use crate::local_prompts::{IncomingPromptRequest, IncomingPromptSource};

#[derive(Default)]
struct StylosActivityCounters {
    status_publish_count: AtomicU64,
    status_publish_total_us: AtomicU64,
    status_publish_max_us: AtomicU64,
    query_request_count: AtomicU64,
    query_request_total_us: AtomicU64,
    query_request_max_us: AtomicU64,
    cmd_event_count: AtomicU64,
    prompt_event_count: AtomicU64,
    event_message_count: AtomicU64,
}

impl StylosActivityCounters {
    fn record_status_publish(&self, elapsed: Duration) {
        self.status_publish_count.fetch_add(1, Ordering::Relaxed);
        let us = elapsed.as_micros() as u64;
        self.status_publish_total_us
            .fetch_add(us, Ordering::Relaxed);
        update_atomic_max(&self.status_publish_max_us, us);
    }

    fn record_query_request(&self, elapsed: Duration) {
        self.query_request_count.fetch_add(1, Ordering::Relaxed);
        let us = elapsed.as_micros() as u64;
        self.query_request_total_us.fetch_add(us, Ordering::Relaxed);
        update_atomic_max(&self.query_request_max_us, us);
    }

    fn snapshot(&self) -> crate::app_runtime::StylosActivitySnapshot {
        crate::app_runtime::StylosActivitySnapshot {
            status_publish_count: self.status_publish_count.load(Ordering::Relaxed),
            status_publish_total_us: self.status_publish_total_us.load(Ordering::Relaxed),
            status_publish_max_us: self.status_publish_max_us.load(Ordering::Relaxed),
            query_request_count: self.query_request_count.load(Ordering::Relaxed),
            query_request_total_us: self.query_request_total_us.load(Ordering::Relaxed),
            query_request_max_us: self.query_request_max_us.load(Ordering::Relaxed),
            cmd_event_count: self.cmd_event_count.load(Ordering::Relaxed),
            prompt_event_count: self.prompt_event_count.load(Ordering::Relaxed),
            event_message_count: self.event_message_count.load(Ordering::Relaxed),
        }
    }
}

fn update_atomic_max(slot: &AtomicU64, value: u64) {
    let mut current = slot.load(Ordering::Relaxed);
    while value > current {
        match slot.compare_exchange(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

#[derive(Clone)]
pub struct StylosQueryContext {
    event_tx: mpsc::UnboundedSender<String>,
    message_inbox: MessageInbox,
    notes_db: Arc<themion_core::db::DbHandle>,
    local_instance: String,
}

impl StylosQueryContext {
    pub fn submit_event(&self, event: String) -> Result<(), String> {
        self.event_tx
            .send(event)
            .map_err(|_| "event queue unavailable".to_string())
    }

    pub fn notes_db(&self) -> &Arc<themion_core::db::DbHandle> {
        &self.notes_db
    }

    pub(crate) fn message_inbox(&self) -> &MessageInbox {
        &self.message_inbox
    }

    pub fn local_instance(&self) -> &str {
        &self.local_instance
    }
}

#[derive(Clone)]
pub struct StylosToolBridge {
    realm: String,
    instance: String,
    session: Arc<zenoh::Session>,
}

impl StylosToolBridge {
    pub async fn invoke(
        &self,
        local_agent_id: Option<&str>,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<String> {
        let reply = match name {
            "stylos_query_agents_alive" => {
                let exclude_self = optional_bool(&args, "exclude_self").unwrap_or(true);
                serde_json::to_value(
                    self.query_discovery::<serde_json::Value>(
                        &format!("stylos/{}/themion/query/agents/alive", self.realm),
                        None,
                        exclude_self,
                    )
                    .await?,
                )?
            }
            "stylos_query_agents_free" => {
                let exclude_self = optional_bool(&args, "exclude_self").unwrap_or(true);
                serde_json::to_value(
                    self.query_discovery::<serde_json::Value>(
                        &format!("stylos/{}/themion/query/agents/free", self.realm),
                        None,
                        exclude_self,
                    )
                    .await?,
                )?
            }
            "stylos_query_agents_git" => {
                let req = serde_json::from_value::<GitQueryRequest>(args.clone())?;
                let exclude_self = req.exclude_self.unwrap_or(true);
                let payload = serde_cbor::to_vec(&req)?;
                serde_json::to_value(
                    self.query_discovery::<serde_json::Value>(
                        &format!("stylos/{}/themion/query/agents/git", self.realm),
                        Some(payload),
                        exclude_self,
                    )
                    .await?,
                )?
            }
            "stylos_query_nodes" => serde_json::to_value(self.query_zenoh_nodes().await?)?,
            "stylos_query_status" => {
                let instance = required_string(&args, "instance")?;
                let req = StatusFilterRequest {
                    agent_id: optional_string(&args, "agent_id"),
                    role: optional_string(&args, "role"),
                };
                serde_json::to_value(
                    self.query_instance::<ToolStatusReply, _>(&instance, "status", Some(&req))
                        .await?,
                )?
            }
            "stylos_send_message" => {
                let instance = required_string(&args, "instance")?;
                let req = MessageRequest {
                    to_agent_id: optional_string(&args, "to_agent_id")
                        .or_else(|| optional_string(&args, "agent_id"))
                        .map(|value| normalize_primary_agent_id(&value).to_string())
                        .unwrap_or_else(|| PRIMARY_AGENT_ID.to_string()),
                    message: required_string(&args, "message")?,
                    request_id: optional_string(&args, "request_id"),
                    from: Some(self.instance.clone()),
                    from_agent_id: local_agent_id.map(str::to_string),
                    to: Some(instance.clone()),
                };
                serde_json::to_value(
                    self.query_instance::<MessageReply, _>(&instance, "messages/send", Some(&req))
                        .await?,
                )?
            }
            "board_create_note" => {
                let instance = required_string(&args, "to_instance")?;
                let req = NoteRequest {
                    to_agent_id: required_string(&args, "to_agent_id")?,
                    body: required_string(&args, "body")?,
                    column: optional_string(&args, "column"),
                    note_kind: optional_string(&args, "note_kind"),
                    origin_note_id: optional_string(&args, "origin_note_id"),
                    request_id: optional_string(&args, "request_id"),
                    from: Some(self.instance.clone()),
                    from_agent_id: local_agent_id.map(str::to_string),
                    to: Some(instance.clone()),
                };
                serde_json::to_value(
                    self.query_instance::<NoteReply, _>(&instance, "notes/request", Some(&req))
                        .await?,
                )?
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
        let mut peer_zids: Vec<String> =
            info.peers_zid().await.map(|zid| zid.to_string()).collect();
        let mut router_zids: Vec<String> = info
            .routers_zid()
            .await
            .map(|zid| zid.to_string())
            .collect();
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

    async fn query_instance<T, P>(
        &self,
        instance: &str,
        leaf: &str,
        payload: Option<&P>,
    ) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
        P: Serialize,
    {
        let key = format!(
            "stylos/{}/themion/instances/{}/query/{}",
            self.realm, instance, leaf
        );
        let encoded_payload = match payload {
            Some(payload) => Some(serde_cbor::to_vec(payload)?),
            None => None,
        };
        let mut builder = self
            .session
            .get(&key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS));
        if let Some(payload) = encoded_payload {
            builder = builder
                .payload(payload)
                .encoding(Encoding::APPLICATION_CBOR);
        }
        let replies = builder.await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut stream = replies.into_stream();
        let mut decoded = None;
        loop {
            match tokio::time::timeout(
                Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS),
                stream.next(),
            )
            .await
            {
                Ok(Some(reply)) => {
                    let sample = reply
                        .into_result()
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
                    let value = serde_cbor::from_slice::<T>(sample.payload().to_bytes().as_ref())?;
                    if decoded.is_some() {
                        anyhow::bail!(
                            "protocol error: multiple replies for direct Stylos query key {key}"
                        );
                    }
                    decoded = Some(value);
                }
                Ok(None) | Err(_) => break,
            }
        }
        decoded.ok_or_else(|| {
            anyhow::anyhow!("timeout/offline: no responder for Stylos query key {key}")
        })
    }
}

impl StylosToolBridge {
    async fn query_discovery<T>(
        &self,
        key: &str,
        payload: Option<Vec<u8>>,
        exclude_self: bool,
    ) -> anyhow::Result<Vec<T>>
    where
        T: for<'de> Deserialize<'de> + DiscoveryInstanceOwned,
    {
        let mut builder = self
            .session
            .get(key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS));
        if let Some(payload) = payload {
            builder = builder
                .payload(payload)
                .encoding(Encoding::APPLICATION_CBOR);
        }
        let replies = builder.await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut out = Vec::new();
        let mut stream = replies.into_stream();
        loop {
            match tokio::time::timeout(
                Duration::from_millis(DISCOVERY_QUERY_TIMEOUT_MS),
                stream.next(),
            )
            .await
            {
                Ok(Some(reply)) => {
                    let sample = match reply.into_result() {
                        Ok(sample) => sample,
                        Err(err) => return Err(anyhow::anyhow!(err.to_string())),
                    };
                    let decoded =
                        serde_cbor::from_slice::<T>(sample.payload().to_bytes().as_ref())?;
                    if exclude_self && decoded.instance() == self.instance {
                        continue;
                    }
                    out.push(decoded);
                }
                Ok(None) | Err(_) => break,
            }
        }
        Ok(out)
    }
}

trait DiscoveryInstanceOwned {
    fn instance(&self) -> &str;
}

impl DiscoveryInstanceOwned for serde_json::Value {
    fn instance(&self) -> &str {
        self.get("instance").and_then(|v| v.as_str()).unwrap_or("")
    }
}

fn required_string(args: &serde_json::Value, field: &str) -> anyhow::Result<String> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing {field}"))
}

fn optional_string(args: &serde_json::Value, field: &str) -> Option<String> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_bool(args: &serde_json::Value, field: &str) -> Option<bool> {
    args.get(field).and_then(|v| v.as_bool())
}

pub struct StylosHandle {
    state: StylosRuntimeState,
    session: Option<Arc<zenoh::Session>>,
    status_task: Option<JoinHandle<()>>,
    queryable_task: Option<JoinHandle<()>>,
    cmd_task: Option<JoinHandle<()>>,
    cmd_rx: Option<mpsc::UnboundedReceiver<StylosCmdRequest>>,
    prompt_rx: Option<mpsc::UnboundedReceiver<IncomingPromptRequest>>,
    event_rx: Option<mpsc::UnboundedReceiver<String>>,
    query_context: StylosQueryContext,
    activity_counters: Arc<StylosActivityCounters>,
}

impl StylosHandle {
    pub fn off() -> Self {
        let (_prompt_tx, prompt_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let notes_db = themion_core::db::DbHandle::open_in_memory().expect("in-memory notes db");
        Self {
            state: StylosRuntimeState::Off,
            session: None,
            status_task: None,
            queryable_task: None,
            cmd_task: None,
            cmd_rx: None,
            prompt_rx: Some(prompt_rx),
            event_rx: Some(event_rx),
            query_context: StylosQueryContext {
                event_tx,
                message_inbox: MessageInbox::default(),
                notes_db,
                local_instance: String::new(),
            },
            activity_counters: Arc::new(StylosActivityCounters::default()),
        }
    }

    pub fn state(&self) -> &StylosRuntimeState {
        &self.state
    }

    pub fn take_cmd_rx(&mut self) -> Option<mpsc::UnboundedReceiver<StylosCmdRequest>> {
        self.cmd_rx.take()
    }

    pub fn take_prompt_rx(&mut self) -> Option<mpsc::UnboundedReceiver<IncomingPromptRequest>> {
        self.prompt_rx.take()
    }

    pub fn take_event_rx(&mut self) -> Option<mpsc::UnboundedReceiver<String>> {
        self.event_rx.take()
    }

    pub fn query_context(&self) -> StylosQueryContext {
        self.query_context.clone()
    }

    pub fn activity_snapshot(&self) -> Option<crate::app_runtime::StylosActivitySnapshot> {
        match self.state {
            StylosRuntimeState::Active { .. } => Some(self.activity_counters.snapshot()),
            _ => None,
        }
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
struct ToolStatusReply {
    found: bool,
    instance: String,
    session_id: String,
    startup_project_dir: String,
    agents: Vec<serde_json::Value>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GitQueryRequest {
    remote: Option<String>,
    exclude_self: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MessageRequest {
    to_agent_id: String,
    message: String,
    request_id: Option<String>,
    from: Option<String>,
    from_agent_id: Option<String>,
    to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MessageReply {
    accepted: bool,
    agent_id: String,
    request_id: Option<String>,
    correlation_id: Option<String>,
    reason: Option<String>,
    delivery_state: String,
    to_instance: String,
    to_agent_id: String,
    queue_position: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct InboxMessage {
    pub correlation_id: String,
    pub request_id: Option<String>,
    pub from_instance: String,
    pub from_agent_id: Option<String>,
    pub to_instance: String,
    pub to_agent_id: String,
    pub message: String,
    pub enqueued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MessageInbox {
    inner: Arc<std::sync::Mutex<HashMap<String, VecDeque<InboxMessage>>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct InboxEnqueueResult {
    pub correlation_id: String,
    pub queue_position: usize,
}

impl MessageInbox {
    pub(crate) fn enqueue(
        &self,
        to_agent_id: String,
        request_id: Option<String>,
        from_instance: String,
        from_agent_id: Option<String>,
        to_instance: String,
        message: String,
    ) -> Result<InboxEnqueueResult, String> {
        let now_ms = unix_epoch_now_ms();
        let mut inner = self.inner.lock().expect("message inbox lock");
        let queue = inner.entry(to_agent_id.clone()).or_default();
        queue.retain(|item| item.expires_at_ms > now_ms);
        if queue.len() >= MAX_INBOX_MESSAGES_PER_AGENT {
            return Err("queue_full".to_string());
        }
        let correlation_id = format!("message-{}", Uuid::new_v4());
        queue.push_back(InboxMessage {
            correlation_id: correlation_id.clone(),
            request_id,
            from_instance,
            from_agent_id,
            to_instance,
            to_agent_id,
            message,
            enqueued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(INBOX_MESSAGE_TTL_MS),
        });
        Ok(InboxEnqueueResult {
            correlation_id,
            queue_position: queue.len(),
        })
    }

    pub(crate) fn pop_next(&self, to_agent_id: &str) -> Option<InboxMessage> {
        let now_ms = unix_epoch_now_ms();
        let mut inner = self.inner.lock().expect("message inbox lock");
        let queue = inner.get_mut(to_agent_id)?;
        while let Some(front) = queue.front() {
            if front.expires_at_ms > now_ms {
                break;
            }
            queue.pop_front();
        }
        let item = queue.pop_front();
        if queue.is_empty() {
            inner.remove(to_agent_id);
        }
        item
    }

    pub(crate) fn fail_all_for_agent(&self, to_agent_id: &str) -> Vec<InboxMessage> {
        self.inner
            .lock()
            .expect("message inbox lock")
            .remove(to_agent_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub(crate) fn count_for_agent(&self, to_agent_id: &str) -> usize {
        let now_ms = unix_epoch_now_ms();
        let mut inner = self.inner.lock().expect("message inbox lock");
        let Some(queue) = inner.get_mut(to_agent_id) else {
            return 0;
        };
        queue.retain(|item| item.expires_at_ms > now_ms);
        let len = queue.len();
        if len == 0 {
            inner.remove(to_agent_id);
        }
        len
    }
}

fn unix_epoch_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NoteRequest {
    to_agent_id: String,
    body: String,
    column: Option<String>,
    note_kind: Option<String>,
    origin_note_id: Option<String>,
    request_id: Option<String>,
    from: Option<String>,
    from_agent_id: Option<String>,
    to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NoteReply {
    accepted: bool,
    agent_id: String,
    request_id: Option<String>,
    note_id: Option<String>,
    note_slug: Option<String>,
    reason: Option<String>,
}

pub async fn start(
    settings: &StylosConfig,
    session: &Session,
    project_dir: &PathBuf,
    notes_db: Arc<themion_core::db::DbHandle>,
    network_domain: DomainHandle,
    shared_status_hub: Option<SharedStylosStatusHub>,
) -> StylosHandle {
    if !settings.enabled() {
        return StylosHandle::off();
    }

    match start_inner(
        settings,
        session,
        project_dir,
        notes_db,
        network_domain,
        shared_status_hub,
    )
    .await
    {
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
    _project_dir: &PathBuf,
    notes_db: Arc<themion_core::db::DbHandle>,
    network_domain: DomainHandle,
    shared_status_hub: Option<SharedStylosStatusHub>,
) -> Result<StylosHandle, String> {
    let key_instance = crate::instance_id::derive_local_instance_id();
    let identity_instance = key_instance
        .split_once(':')
        .map(|(hostname, _)| hostname.to_string())
        .unwrap_or_else(|| "local".to_string());
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
        stylos::open_session(&cfg, &overrides)
            .await
            .map_err(|e| e.to_string())?,
    );

    let ct = CancellationToken::new();
    let (_prompt_tx, prompt_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let message_inbox = MessageInbox::default();
    let activity_counters = Arc::new(StylosActivityCounters::default());
    let snapshot_provider = shared_status_hub
        .unwrap_or_else(SharedStylosStatusHub::new)
        .provider();
    let query_context = StylosQueryContext {
        event_tx,
        message_inbox: message_inbox.clone(),
        notes_db,
        local_instance: key_instance.clone(),
    };

    let status_ct = ct.clone();
    let status_session = session_handle.clone();
    let status_key = format!("stylos/{}/themion/{}/status", realm, key_instance);
    let snapshot_provider_for_status_task = snapshot_provider.clone();
    let status_mode = mode.clone();
    let status_realm = realm.clone();
    let status_instance = key_instance.clone();
    let status_activity_counters = activity_counters.clone();
    let status_task = network_domain.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = status_ct.cancelled() => break,
                _ = interval.tick() => {
                    let publish_started = Instant::now();
                    let snapshot = snapshot_provider_for_status_task().await;
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
                    status_activity_counters.record_status_publish(publish_started.elapsed());
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
    let q_status_key = format!(
        "stylos/{}/themion/instances/{}/query/status",
        realm, key_instance
    );
    let q_message_key = format!(
        "stylos/{}/themion/instances/{}/query/messages/send",
        realm, key_instance
    );
    let q_note_key = format!(
        "stylos/{}/themion/instances/{}/query/notes/request",
        realm, key_instance
    );
    let info = ThemionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        instance: key_instance.clone(),
        realm: realm.clone(),
        mode: mode.clone(),
        profile: session.active_profile.clone(),
        model: session.model.clone(),
    };
    let query_status_provider = snapshot_provider.clone();
    let query_context_for_delivery = query_context.clone();
    let query_instance = key_instance.clone();
    let query_session_id = session.id.to_string();
    let query_activity_counters = activity_counters.clone();
    let queryable_task = network_domain.spawn(async move {
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
        let message_queryable = match q_session.declare_queryable(&q_message_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let note_queryable = match q_session.declare_queryable(&q_note_key).await {
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
                        let query_started = Instant::now();
                        if let Some(reply) = build_discovery_reply(&query_status_provider, &query_instance, &query_session_id, DiscoveryMode::Alive).await {
                            let _ = reply_cbor(query, q_alive_key.clone(), &reply).await;
                        }
                        query_activity_counters.record_query_request(query_started.elapsed());
                    }
                    Err(_) => break,
                },
                res = free_queryable.recv_async() => match res {
                    Ok(query) => {
                        let query_started = Instant::now();
                        if let Some(reply) = build_discovery_reply(&query_status_provider, &query_instance, &query_session_id, DiscoveryMode::Free).await {
                            let _ = reply_cbor(query, q_free_key.clone(), &reply).await;
                        }
                        query_activity_counters.record_query_request(query_started.elapsed());
                    }
                    Err(_) => break,
                },
                res = git_queryable.recv_async() => match res {
                    Ok(query) => {
                        let query_started = Instant::now();
                        let req = parse_cbor_payload::<GitQueryRequest>(&query);
                        if let Some(reply) = build_git_reply(&query_status_provider, &query_instance, &query_session_id, req).await {
                            let _ = reply_cbor(query, q_git_key.clone(), &reply).await;
                        }
                        query_activity_counters.record_query_request(query_started.elapsed());
                    }
                    Err(_) => break,
                },
                res = status_queryable.recv_async() => match res {
                    Ok(query) => {
                        let query_started = Instant::now();
                        let req = parse_cbor_payload::<StatusFilterRequest>(&query);
                        let reply = build_status_reply(&query_status_provider, &query_instance, &query_session_id, req).await;
                        let _ = reply_cbor(query, q_status_key.clone(), &reply).await;
                        query_activity_counters.record_query_request(query_started.elapsed());
                    }
                    Err(_) => break,
                },
                res = message_queryable.recv_async() => match res {
                    Ok(query) => {
                        let query_started = Instant::now();
                        let reply = handle_send_message_query(&query_status_provider, &query_context_for_delivery, parse_cbor_payload::<MessageRequest>(&query)).await;
                        let _ = reply_cbor(query, q_message_key.clone(), &reply).await;
                        query_activity_counters.record_query_request(query_started.elapsed());
                    }
                    Err(_) => break,
                },
                res = note_queryable.recv_async() => match res {
                    Ok(query) => {
                        let query_started = Instant::now();
                        let reply = handle_note_delivery_query(&query_status_provider, &query_context_for_delivery, parse_cbor_payload::<NoteRequest>(&query)).await;
                        let _ = reply_cbor(query, q_note_key.clone(), &reply).await;
                        query_activity_counters.record_query_request(query_started.elapsed());
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
    let cmd_activity_counters = activity_counters.clone();
    let cmd_task = network_domain.spawn(async move {
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
                        cmd_activity_counters.cmd_event_count.fetch_add(1, Ordering::Relaxed);
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
        session: Some(session_handle),
        status_task: Some(status_task),
        queryable_task: Some(queryable_task),
        cmd_task: Some(cmd_task),
        cmd_rx: Some(cmd_rx),
        prompt_rx: Some(prompt_rx),
        event_rx: Some(event_rx),
        query_context,
        activity_counters,
    })
}

#[derive(Clone, Copy)]
enum DiscoveryMode {
    Alive,
    Free,
}

async fn build_discovery_reply(
    snapshot_provider: &StylosSnapshotProvider,
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
    snapshot_provider: &StylosSnapshotProvider,
    instance: &str,
    session_id: &str,
    req: Option<GitQueryRequest>,
) -> Option<DiscoveryReply> {
    let snapshot = current_snapshot(snapshot_provider).await?;
    let mut agents = build_queryable_agents(snapshot.agents);
    let requested = req.and_then(|r| r.remote);
    let requested_key = requested.as_deref().and_then(normalize_git_remote);
    agents.retain(|agent| {
        git_agent_matches_request(agent, requested.as_deref(), requested_key.as_deref())
    });
    Some(DiscoveryReply {
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        agents,
    })
}

async fn build_status_reply(
    snapshot_provider: &StylosSnapshotProvider,
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
    if let Some(req) = req {
        if let Some(agent_id) = req.agent_id {
            let agent_id = normalize_primary_agent_id(&agent_id);
            agents.retain(|agent| agent.agent_id == agent_id);
        }
        if let Some(role) = req.role {
            let role = normalize_primary_role(&role);
            agents.retain(|agent| agent.roles.iter().any(|r| r == role));
        }
    }
    let found = !agents.is_empty();
    StatusReply {
        found,
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        startup_project_dir: snapshot.startup_project_dir,
        agents,
        error: if found {
            None
        } else {
            Some("not_found".to_string())
        },
    }
}

async fn handle_send_message_query(
    snapshot_provider: &StylosSnapshotProvider,
    query_context: &StylosQueryContext,
    req: Option<MessageRequest>,
) -> MessageReply {
    let Some(req) = req else {
        return rejected_message_reply(
            String::new(),
            None,
            "invalid_request",
            query_context.local_instance().to_string(),
        );
    };
    let normalized_to_agent_id = normalize_primary_agent_id(&req.to_agent_id).to_string();
    let to_instance = req
        .to
        .clone()
        .unwrap_or_else(|| query_context.local_instance().to_string());
    let Some(snapshot) = current_snapshot(snapshot_provider).await else {
        return rejected_message_reply(
            normalized_to_agent_id,
            req.request_id,
            "snapshot_unavailable",
            to_instance,
        );
    };
    let agent = snapshot
        .agents
        .into_iter()
        .find(|a| a.agent_id == normalized_to_agent_id);
    let Some(agent) = agent else {
        return rejected_message_reply(
            normalized_to_agent_id,
            req.request_id,
            "not_found",
            to_instance,
        );
    };

    let sender = render_instance_identifier(req.from.as_deref());
    let target = render_instance_identifier(req.to.as_deref());
    let enqueue_result = query_context.message_inbox().enqueue(
        agent.agent_id.clone(),
        req.request_id.clone(),
        sender.clone(),
        req.from_agent_id.clone(),
        target.clone(),
        req.message.clone(),
    );
    match enqueue_result {
        Ok(queued) => {
            let _ = query_context.submit_event(format!(
                "Stylos message queued correlation_id={} from={} from_agent_id={} to={} to_agent_id={} queue_position={}",
                queued.correlation_id,
                sender,
                req.from_agent_id.as_deref().unwrap_or("unknown"),
                target,
                agent.agent_id,
                queued.queue_position,
            ));
            MessageReply {
                accepted: true,
                agent_id: agent.agent_id.clone(),
                request_id: req.request_id,
                correlation_id: Some(queued.correlation_id),
                reason: None,
                delivery_state: "queued".to_string(),
                to_instance: target,
                to_agent_id: agent.agent_id,
                queue_position: Some(queued.queue_position),
            }
        }
        Err(reason) => {
            let _ = query_context.submit_event(format!(
                "Stylos message rejected reason={} from={} from_agent_id={} to={} to_agent_id={}",
                reason,
                sender,
                req.from_agent_id.as_deref().unwrap_or("unknown"),
                target,
                agent.agent_id,
            ));
            rejected_message_reply(agent.agent_id, req.request_id, reason, target)
        }
    }
}

fn rejected_message_reply(
    agent_id: String,
    request_id: Option<String>,
    reason: impl Into<String>,
    to_instance: String,
) -> MessageReply {
    MessageReply {
        accepted: false,
        to_agent_id: agent_id.clone(),
        agent_id,
        request_id,
        correlation_id: None,
        reason: Some(reason.into()),
        delivery_state: "rejected".to_string(),
        to_instance,
        queue_position: None,
    }
}

async fn handle_note_delivery_query(
    snapshot_provider: &StylosSnapshotProvider,
    query_context: &StylosQueryContext,
    req: Option<NoteRequest>,
) -> NoteReply {
    let Some(req) = req else {
        return NoteReply {
            accepted: false,
            agent_id: String::new(),
            request_id: None,
            note_id: None,
            note_slug: None,
            reason: Some("invalid_request".to_string()),
        };
    };
    let normalized_to_agent_id = normalize_primary_agent_id(&req.to_agent_id).to_string();
    let Some(snapshot) = current_snapshot(snapshot_provider).await else {
        return NoteReply {
            accepted: false,
            agent_id: normalized_to_agent_id.clone(),
            request_id: req.request_id,
            note_id: None,
            note_slug: None,
            reason: Some("snapshot_unavailable".to_string()),
        };
    };
    let Some(agent) = snapshot
        .agents
        .into_iter()
        .find(|a| a.agent_id == normalized_to_agent_id)
    else {
        return NoteReply {
            accepted: false,
            agent_id: normalized_to_agent_id.clone(),
            request_id: req.request_id,
            note_id: None,
            note_slug: None,
            reason: Some("not_found".to_string()),
        };
    };
    let note_id = Uuid::new_v4().to_string();
    let column = match req.column.as_deref().unwrap_or("todo") {
        "todo" => NoteColumn::Todo,
        "blocked" => NoteColumn::Blocked,
        _ => {
            return NoteReply {
                accepted: false,
                agent_id: agent.agent_id,
                request_id: req.request_id,
                note_id: None,
                note_slug: None,
                reason: Some("invalid_column".to_string()),
            }
        }
    };
    let note_kind = match req.note_kind.as_deref().unwrap_or("work_request") {
        "work_request" => NoteKind::WorkRequest,
        "done_mention" => NoteKind::DoneMention,
        _ => {
            return NoteReply {
                accepted: false,
                agent_id: agent.agent_id,
                request_id: req.request_id,
                note_id: None,
                note_slug: None,
                reason: Some("invalid_note_kind".to_string()),
            }
        }
    };
    let created = query_context.notes_db().create_board_note(CreateNoteArgs {
        note_id: note_id.clone(),
        note_kind,
        column,
        origin_note_id: req.origin_note_id.clone(),
        from_instance: req.from.clone(),
        from_agent_id: req.from_agent_id.clone(),
        to_instance: query_context.local_instance().to_string(),
        to_agent_id: agent.agent_id.clone(),
        body: req.body.clone(),
        meta_json: None,
    });
    match created {
        Ok(note) => {
            let _ = query_context.submit_event(format!(
                "Board note posted note_slug={} column={}",
                note.note_slug,
                note.column.as_str(),
            ));
            NoteReply {
                accepted: true,
                agent_id: agent.agent_id,
                request_id: req.request_id,
                note_id: Some(note_id),
                note_slug: Some(note.note_slug),
                reason: None,
            }
        }
        Err(err) => NoteReply {
            accepted: false,
            agent_id: agent.agent_id,
            request_id: req.request_id,
            note_id: None,
            note_slug: None,
            reason: Some(err.to_string()),
        },
    }
}

async fn current_snapshot(
    snapshot_provider: &StylosSnapshotProvider,
) -> Option<StylosStatusSnapshot> {
    Some(snapshot_provider().await)
}

fn build_queryable_agents(
    agents: Vec<StylosAgentStatusSnapshot>,
) -> Vec<StylosQueryableAgentSnapshot> {
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

fn git_agent_matches_request(
    agent: &StylosQueryableAgentSnapshot,
    requested: Option<&str>,
    requested_key: Option<&str>,
) -> bool {
    if !agent.project_dir_is_git_repo {
        return false;
    }
    match (requested, requested_key) {
        (None, _) => true,
        (Some(raw), Some(key)) => {
            agent.git_repo_keys.iter().any(|candidate| candidate == key)
                || agent.git_remotes.iter().any(|remote| remote == raw)
        }
        (Some(raw), None) => agent.git_remotes.iter().any(|remote| remote == raw),
    }
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
        .or_else(|| trimmed.strip_prefix("ssh://"))
        .unwrap_or(trimmed);

    let (host, path) = without_scheme.split_once('/')?;
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

fn parse_cbor_payload<T: for<'de> Deserialize<'de>>(query: &zenoh::query::Query) -> Option<T> {
    let payload = query.payload()?;
    let bytes = payload.to_bytes();
    if bytes.is_empty() {
        return None;
    }
    serde_cbor::from_slice(bytes.as_ref()).ok()
}

async fn reply_cbor<T: Serialize>(
    query: zenoh::query::Query,
    key: String,
    payload: &T,
) -> Result<(), zenoh::Error> {
    let bytes = serde_cbor::to_vec(payload).unwrap_or_default();
    query
        .reply(key, bytes)
        .encoding(Encoding::APPLICATION_CBOR)
        .await
}

fn render_instance_identifier(instance: Option<&str>) -> String {
    instance
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("external")
        .to_string()
}

impl InboxMessage {
    pub(crate) fn into_incoming_prompt(self) -> IncomingPromptRequest {
        let prompt = if crate::local_prompts::is_peer_message_prompt(&self.message) {
            self.message.clone()
        } else {
            build_peer_message_prompt(
                &self.from_instance,
                &self.to_instance,
                &self.to_agent_id,
                &self.message,
                self.from_agent_id.as_deref(),
            )
        };
        IncomingPromptRequest {
            prompt,
            source: IncomingPromptSource::RemoteStylos,
            agent_id: Some(self.to_agent_id.clone()),
            request_id: self.request_id.clone(),
            from: Some(self.from_instance),
            from_agent_id: self.from_agent_id,
            to: Some(self.to_instance),
            to_agent_id: Some(self.to_agent_id),
        }
    }
}

fn build_peer_message_prompt(
    sender: &str,
    target: &str,
    local_agent_id: &str,
    message: &str,
    sender_agent_id: Option<&str>,
) -> String {
    let from_agent = sender_agent_id.unwrap_or("unknown");
    format!(
        "## Message context
You received a volatile Stylos peer message. This wrapper provides handling guidance plus delivery metadata. The sender's actual message is in the body section below.

## Handling instructions
- Read the message body as the sender's request, instruction, question, or status update.
- If the body asks you to do work or gives you a job, do the requested work when it fits your role and available tools.
- Reply with `stylos_send_message` when you have a useful answer, result, blocker, correction, or follow-up question.
- Do not send empty acknowledgements or thank-you-only messages.
- Prefer one concise useful response rather than a conversational back-and-forth.
- If your response completes the exchange and no further reply should be sent, include ***QRU***.
- Treat received ***QRU*** in the body as a strong signal that no reply is needed unless there is important corrective information.

## Delivery metadata
The next line is machine-readable routing metadata for this message. It is not the sender's message body.
type=peer_message from={sender} from_agent_id={from_agent} to={target} to_agent_id={local_agent_id}

## Message body
{message}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_string_rejects_blank_values() {
        let args = serde_json::json!({"field": "   "});
        assert!(required_string(&args, "field").is_err());
    }

    #[test]
    fn optional_string_ignores_blank_values() {
        let args = serde_json::json!({"field": "   "});
        assert_eq!(optional_string(&args, "field"), None);
    }

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

    #[test]
    fn normalizes_repo_key_without_scheme() {
        assert_eq!(
            normalize_git_remote("github.com/example/themion").as_deref(),
            Some("github.com/example/themion")
        );
    }

    #[test]
    fn discovery_instance_owned_reads_instance_from_payload() {
        let value = serde_json::json!({"instance": "vm-02:123"});
        assert_eq!(value.instance(), "vm-02:123");

        let missing = serde_json::json!({"agents": []});
        assert_eq!(missing.instance(), "");
    }

    fn test_git_agent(remotes: &[&str], is_repo: bool) -> StylosQueryableAgentSnapshot {
        let git_remotes: Vec<String> = remotes.iter().map(|remote| (*remote).to_string()).collect();
        StylosQueryableAgentSnapshot {
            agent_id: "master".to_string(),
            label: "master".to_string(),
            roles: vec!["master".to_string()],
            session_id: "session".to_string(),
            activity_status: "idle".to_string(),
            activity_status_changed_at_ms: 0,
            project_dir: "/tmp/repo".to_string(),
            project_dir_is_git_repo: is_repo,
            git_repo_keys: derive_git_repo_keys(&git_remotes),
            git_remotes,
            provider: "provider".to_string(),
            model: "model".to_string(),
            active_profile: "profile".to_string(),
            workflow: WorkflowState::default(),
            rate_limits: None,
        }
    }

    #[test]
    fn git_query_matches_equivalent_remote_forms() {
        let agent = test_git_agent(&["git@github.com:example/themion.git"], true);
        assert!(git_agent_matches_request(
            &agent,
            Some("git@github.com:example/themion.git"),
            normalize_git_remote("git@github.com:example/themion.git").as_deref(),
        ));
        assert!(git_agent_matches_request(
            &agent,
            Some("https://github.com/example/themion"),
            normalize_git_remote("https://github.com/example/themion").as_deref(),
        ));
        assert!(git_agent_matches_request(
            &agent,
            Some("github.com/example/themion"),
            normalize_git_remote("github.com/example/themion").as_deref(),
        ));
    }

    #[test]
    fn git_query_rejects_non_matching_repo() {
        let agent = test_git_agent(&["git@github.com:example/themion.git"], true);
        assert!(!git_agent_matches_request(
            &agent,
            Some("github.com/example/other"),
            normalize_git_remote("github.com/example/other").as_deref(),
        ));
    }

    #[test]
    fn git_query_falls_back_to_exact_raw_match_when_query_cannot_normalize() {
        let agent = test_git_agent(&["file:///tmp/themion"], true);
        assert!(git_agent_matches_request(
            &agent,
            Some("file:///tmp/themion"),
            None
        ));
        assert!(!git_agent_matches_request(
            &agent,
            Some("file:///tmp/other"),
            None
        ));
    }

    #[test]
    fn git_query_excludes_non_git_agents() {
        let agent = test_git_agent(&["git@github.com:example/themion.git"], false);
        assert!(!git_agent_matches_request(
            &agent,
            Some("github.com/example/themion"),
            normalize_git_remote("github.com/example/themion").as_deref(),
        ));
    }

    #[test]
    fn sender_identity_falls_back_for_external_sender() {
        assert_eq!(render_instance_identifier(None), "external");
    }

    #[test]
    fn instance_identifier_uses_instance_only() {
        assert_eq!(render_instance_identifier(Some("node-1:42")), "node-1:42");
        assert_eq!(render_instance_identifier(None), "external");
    }

    #[test]
    fn peer_prompt_mentions_qru_and_sender() {
        let prompt =
            build_peer_message_prompt("node-1:42", "node-2:77", "master", "hello", Some("smith-1"));
        assert!(prompt.contains("***QRU***"));
        assert!(prompt.contains("## Delivery metadata"));
        assert!(prompt.contains("machine-readable routing metadata"));
        assert!(prompt.contains("type=peer_message from=node-1:42 from_agent_id=smith-1 to=node-2:77 to_agent_id=master"));
        assert!(prompt.contains("## Message context"));
        assert!(crate::local_prompts::is_peer_message_prompt(&prompt));
        assert!(prompt.contains("## Handling instructions"));
        assert!(prompt.contains("## Message body"));
        assert!(prompt.contains("do the requested work"));
        assert!(prompt.contains("stylos_send_message"));
        assert!(prompt.contains("hello"));
    }

    #[test]
    fn queued_peer_prompt_is_not_double_wrapped() {
        let prompt =
            build_peer_message_prompt("node-1:42", "node-2:77", "master", "hello", Some("smith-1"));
        let request = InboxMessage {
            correlation_id: "message-1".to_string(),
            request_id: Some("request-1".to_string()),
            from_instance: "node-1:42".to_string(),
            from_agent_id: Some("smith-1".to_string()),
            to_instance: "node-2:77".to_string(),
            to_agent_id: "master".to_string(),
            message: prompt.clone(),
            enqueued_at_ms: 0,
            expires_at_ms: INBOX_MESSAGE_TTL_MS,
        }
        .into_incoming_prompt();

        assert_eq!(request.prompt, prompt);
    }

    #[test]
    fn note_prompt_mentions_note_identity_and_body() {
        let prompt = crate::local_prompts::build_board_note_prompt(
            "123e4567-e89b-12d3-a456-426614174000",
            "fix-tests-123e4567",
            NoteKind::WorkRequest,
            None,
            Some("node-1:42"),
            Some("master"),
            "node-2:77",
            "worker",
            NoteColumn::Todo,
            "please fix the tests",
            IncomingPromptSource::WatchdogBoardNote,
        );
        assert!(prompt.contains("type=stylos_note"));
        assert!(prompt.contains("note_id=123e4567-e89b-12d3-a456-426614174000"));
        assert!(prompt.contains("note_slug=fix-tests-123e4567"));
        assert!(prompt.contains("from=node-1:42"));
        assert!(prompt.contains("from_agent_id=master"));
        assert!(prompt.contains("to=node-2:77"));
        assert!(prompt.contains("to_agent_id=worker"));
        assert!(prompt.contains("note_kind=work_request"));
        assert!(prompt.contains("column=todo"));
        assert!(
            prompt.contains("I found that you have a pending note to handle. Below is that note.")
        );
        assert!(prompt.contains(
            "Note body:
please fix the tests"
        ));
    }
}

pub fn tool_bridge(handle: &StylosHandle) -> Option<StylosToolBridge> {
    match handle.state() {
        StylosRuntimeState::Active {
            realm, instance, ..
        } => Some(StylosToolBridge {
            realm: realm.clone(),
            instance: instance.clone(),
            session: handle.session.as_ref()?.clone(),
        }),
        _ => None,
    }
}

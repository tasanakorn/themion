#![cfg(feature = "stylos")]

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
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zenoh::bytes::Encoding;
use zenoh::qos::CongestionControl;

use crate::config::StylosConfig;
use crate::Session;

const GIT_STATUS_TTL: Duration = Duration::from_secs(30);

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

type StylosSnapshotFuture = Pin<Box<dyn Future<Output = StylosStatusSnapshot> + Send>>;
type StylosSnapshotProvider = Arc<dyn Fn() -> StylosSnapshotFuture + Send + Sync>;

pub struct StylosHandle {
    state: StylosRuntimeState,
    session: Option<Arc<zenoh::Session>>,
    status_task: Option<JoinHandle<()>>,
    queryable_task: Option<JoinHandle<()>>,
    cmd_task: Option<JoinHandle<()>>,
    cmd_rx: Option<mpsc::UnboundedReceiver<StylosCmdRequest>>,
    snapshot_provider: Arc<RwLock<Option<StylosSnapshotProvider>>>,
}

impl StylosHandle {
    pub fn off() -> Self {
        Self {
            state: StylosRuntimeState::Off,
            session: None,
            status_task: None,
            queryable_task: None,
            cmd_task: None,
            cmd_rx: None,
            snapshot_provider: Arc::new(RwLock::new(None)),
        }
    }

    pub fn state(&self) -> &StylosRuntimeState {
        &self.state
    }

    pub fn take_cmd_rx(&mut self) -> Option<mpsc::UnboundedReceiver<StylosCmdRequest>> {
        self.cmd_rx.take()
    }

    pub async fn set_snapshot_provider(&self, provider: StylosSnapshotProvider) {
        *self.snapshot_provider.write().await = Some(provider);
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
        Err(err) => StylosHandle {
            state: StylosRuntimeState::Error(err),
            session: None,
            status_task: None,
            queryable_task: None,
            cmd_task: None,
            cmd_rx: None,
            snapshot_provider: Arc::new(RwLock::new(None)),
        },
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
    let q_key = format!("stylos/{}/themion/{}/info", realm, key_instance);
    let info = ThemionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        instance: key_instance.clone(),
        realm: realm.clone(),
        mode: mode.clone(),
        profile: session.active_profile.clone(),
        model: session.model.clone(),
    };
    let queryable_task = tokio::spawn(async move {
        let queryable = match q_session.declare_queryable(&q_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let payload = serde_json::to_vec(&info).unwrap_or_default();
        loop {
            tokio::select! {
                _ = q_ct.cancelled() => break,
                res = queryable.recv_async() => match res {
                    Ok(query) => {
                        let _ = query
                            .reply(q_key.clone(), payload.clone())
                            .encoding(Encoding::APPLICATION_JSON)
                            .await;
                    }
                    Err(_) => break,
                }
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
        session: Some(session_handle),
        status_task: Some(status_task),
        queryable_task: Some(queryable_task),
        cmd_task: Some(cmd_task),
        cmd_rx: Some(cmd_rx),
        snapshot_provider,
    })
}

#[derive(Clone, Debug)]
pub struct GitProjectStatus {
    pub is_repo: bool,
    pub remotes: Vec<String>,
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

    let mut urls = std::collections::BTreeSet::<String>::new();
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

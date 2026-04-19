#![cfg(feature = "stylos")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use stylos_config::{Endpoints, IdentitySection, StylosConfig as SessionConfig, ZenohSection};
use stylos_session::SessionOverrides;
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::workflow::WorkflowState;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zenoh::bytes::Encoding;
use zenoh::qos::CongestionControl;

use crate::Session;
use crate::config::StylosConfig;

#[derive(Clone, Debug)]
pub enum StylosRuntimeState {
    Off,
    Active { mode: String, realm: String, instance: String },
    Error(String),
}

#[derive(Clone, Debug)]
pub struct StylosStatusSnapshot {
    pub workflow: WorkflowState,
    pub activity_status: String,
    pub project_dir: String,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub rate_limits: Option<ApiCallRateLimitReport>,
}

impl StylosStatusSnapshot {
    pub fn new(session: &Session, project_dir: &PathBuf) -> Self {
        Self {
            workflow: WorkflowState::default(),
            activity_status: "idle".to_string(),
            project_dir: project_dir.display().to_string(),
            provider: session.provider.clone(),
            model: session.model.clone(),
            active_profile: session.active_profile.clone(),
            rate_limits: None,
        }
    }
}

pub struct StylosHandle {
    state: StylosRuntimeState,
    session: Option<Arc<zenoh::Session>>,
    heartbeat_task: Option<JoinHandle<()>>,
    status_task: Option<JoinHandle<()>>,
    queryable_task: Option<JoinHandle<()>>,
    status_snapshot: Arc<RwLock<StylosStatusSnapshot>>,
}

impl StylosHandle {
    pub fn off() -> Self {
        Self {
            state: StylosRuntimeState::Off,
            session: None,
            heartbeat_task: None,
            status_task: None,
            queryable_task: None,
            status_snapshot: Arc::new(RwLock::new(StylosStatusSnapshot {
                workflow: WorkflowState::default(),
                activity_status: "idle".to_string(),
                project_dir: String::new(),
                provider: String::new(),
                model: String::new(),
                active_profile: String::new(),
                rate_limits: None,
            })),
        }
    }

    pub fn state(&self) -> &StylosRuntimeState {
        &self.state
    }

    pub fn status_snapshot(&self) -> Arc<RwLock<StylosStatusSnapshot>> {
        self.status_snapshot.clone()
    }

    pub async fn shutdown(self) {
        if let Some(task) = self.heartbeat_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.status_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.queryable_task {
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
    profile: String,
    provider: String,
    model: String,
    project_dir: String,
    workflow: WorkflowState,
    activity_status: String,
    rate_limits: Option<ApiCallRateLimitReport>,
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
            heartbeat_task: None,
            status_task: None,
            queryable_task: None,
            status_snapshot: Arc::new(RwLock::new(StylosStatusSnapshot::new(session, project_dir))),
        },
    }
}

async fn start_inner(
    settings: &StylosConfig,
    session: &Session,
    project_dir: &PathBuf,
) -> Result<StylosHandle, String> {
    let hostname = derive_hostname().unwrap_or_else(|| "themion".to_string());
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

    let status_snapshot = Arc::new(RwLock::new(StylosStatusSnapshot::new(session, project_dir)));

    let ct = CancellationToken::new();
    let hb_ct = ct.clone();
    let hb_session = session_handle.clone();
    let hb_key = format!("stylos/{}/themion/{}/heartbeat", realm, key_instance);
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = hb_ct.cancelled() => break,
                _ = interval.tick() => {
                    let _ = hb_session
                        .put(&hb_key, b"alive".to_vec())
                        .encoding(Encoding::APPLICATION_OCTET_STREAM)
                        .congestion_control(CongestionControl::Drop)
                        .await;
                }
            }
        }
    });

    let status_ct = ct.clone();
    let status_session = session_handle.clone();
    let status_key = format!("stylos/{}/themion/{}/status", realm, key_instance);
    let status_snapshot_reader = status_snapshot.clone();
    let status_profile = session.active_profile.clone();
    let status_provider = session.provider.clone();
    let status_model = session.model.clone();
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
                    let snapshot = status_snapshot_reader.read().await.clone();
                    let payload = ThemionStatusPayload {
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        instance: status_instance.clone(),
                        realm: status_realm.clone(),
                        mode: status_mode.clone(),
                        profile: status_profile.clone(),
                        provider: status_provider.clone(),
                        model: status_model.clone(),
                        project_dir: snapshot.project_dir,
                        workflow: snapshot.workflow,
                        activity_status: snapshot.activity_status,
                        rate_limits: snapshot.rate_limits,
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

    Ok(StylosHandle {
        state: StylosRuntimeState::Active { mode, realm, instance: key_instance },
        session: Some(session_handle),
        heartbeat_task: Some(heartbeat_task),
        status_task: Some(status_task),
        queryable_task: Some(queryable_task),
        status_snapshot,
    })
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

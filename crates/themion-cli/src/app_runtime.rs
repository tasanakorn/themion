use crate::config::Config;
use crate::runtime_domains::RuntimeDomains;
use crate::Session;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::db::DbHandle;

pub struct AppRuntimeInit {
    pub cfg: Config,
    pub project_dir_override: Option<PathBuf>,
    pub enable_tui_session: bool,
}


pub struct AppEventChannels<T> {
    pub tx: tokio::sync::mpsc::UnboundedSender<T>,
    pub rx: tokio::sync::mpsc::UnboundedReceiver<T>,
}

impl<T> AppEventChannels<T> {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self { tx, rx }
    }
}

#[cfg(feature = "stylos")]
pub struct StylosEventReceivers {
    pub cmd_rx: tokio::sync::mpsc::UnboundedReceiver<crate::stylos::StylosCmdRequest>,
    pub prompt_rx: tokio::sync::mpsc::UnboundedReceiver<crate::stylos::IncomingPromptRequest>,
    pub event_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
}

pub struct AppRuntime {
    pub runtime_domains: Arc<RuntimeDomains>,
    pub session: Session,
    pub db: Arc<DbHandle>,
    pub project_dir: PathBuf,
    #[cfg(feature = "stylos")]
    pub stylos_config: crate::config::StylosConfig,
}


#[cfg(feature = "stylos")]
pub async fn start_stylos(app_runtime: &AppRuntime) -> anyhow::Result<(crate::stylos::StylosHandle, StylosEventReceivers)> {
    let network_domain = app_runtime.runtime_domains.network();
    let mut handle = match network_domain
        .spawn({
            let stylos_cfg = app_runtime.stylos_config.clone();
            let session = app_runtime.session.clone();
            let project_dir = app_runtime.project_dir.clone();
            let db = app_runtime.db.clone();
            let network_domain = network_domain.clone();
            async move {
                crate::stylos::start(&stylos_cfg, &session, &project_dir, db, network_domain).await
            }
        })
        .await
    {
        Ok(handle) => handle,
        Err(err) => return Err(anyhow::anyhow!("failed to start stylos runtime: {}", err)),
    };

    let receivers = StylosEventReceivers {
        cmd_rx: handle
            .take_cmd_rx()
            .unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().1),
        prompt_rx: handle
            .take_prompt_rx()
            .unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().1),
        event_rx: handle
            .take_event_rx()
            .unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().1),
    };

    Ok((handle, receivers))
}

impl AppRuntime {
    pub fn build(init: AppRuntimeInit) -> anyhow::Result<Self> {
        #[cfg(feature = "stylos")]
        let stylos_config = init.cfg.stylos.clone();

        let runtime_domains = Arc::new(if init.enable_tui_session {
            RuntimeDomains::for_tui_mode()?
        } else {
            RuntimeDomains::for_print_mode()?
        });

        let project_dir = init
            .project_dir_override
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .canonicalize()
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let db = match dirs::data_dir() {
            Some(d) => themion_core::db::open_default_in_data_dir(&d).unwrap_or_else(|e| {
                if init.enable_tui_session {
                    eprintln!("warning: history persistence disabled: {}", e);
                }
                DbHandle::open_in_memory().expect("in-memory db")
            }),
            None => {
                if init.enable_tui_session {
                    eprintln!("warning: history persistence disabled (no data dir)");
                }
                DbHandle::open_in_memory().expect("in-memory db")
            }
        };

        Ok(Self {
            runtime_domains,
            session: Session::from_config(init.cfg),
            db,
            project_dir,
            #[cfg(feature = "stylos")]
            stylos_config,
        })
    }
}




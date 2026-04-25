use anyhow::Result;
use std::future::Future;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDomain {
    Tui,
    Core,
    Network,
    Background,
}

impl RuntimeDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Core => "core",
            Self::Network => "network",
            Self::Background => "background",
        }
    }
}

#[derive(Clone)]
pub struct DomainHandle {
    _name: RuntimeDomain,
    handle: Handle,
}

impl DomainHandle {
    fn new(name: RuntimeDomain, handle: Handle) -> Self {
        Self {
            _name: name,
            handle,
        }
    }

    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(fut)
    }

    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        tokio::task::block_in_place(|| self.handle.block_on(fut))
    }
}

struct OwnedRuntime {
    name: RuntimeDomain,
    runtime: Runtime,
}

impl OwnedRuntime {
    fn handle(&self) -> DomainHandle {
        DomainHandle::new(self.name, self.runtime.handle().clone())
    }
}

pub struct RuntimeDomains {
    _tui_runtime: Option<OwnedRuntime>,
    _core_runtime: OwnedRuntime,
    _network_runtime: OwnedRuntime,
    _background_runtime: Option<OwnedRuntime>,
    tui: Option<DomainHandle>,
    core: DomainHandle,
    #[cfg(feature = "stylos")]
    network: DomainHandle,
    background: Option<DomainHandle>,
}

impl RuntimeDomains {
    pub fn for_tui_mode() -> Result<Self> {
        Self::build(true, true)
    }

    pub fn for_print_mode() -> Result<Self> {
        Self::build(false, false)
    }

    fn build(include_tui: bool, include_background: bool) -> Result<Self> {
        let tui_runtime = if include_tui {
            Some(build_multi_thread_runtime(RuntimeDomain::Tui, 1)?)
        } else {
            None
        };
        let core_runtime = build_multi_thread_runtime(RuntimeDomain::Core, 2)?;
        let network_runtime = build_multi_thread_runtime(RuntimeDomain::Network, 2)?;
        let background_runtime = if include_background {
            Some(build_multi_thread_runtime(RuntimeDomain::Background, 1)?)
        } else {
            None
        };

        let tui = tui_runtime.as_ref().map(OwnedRuntime::handle);
        let core = core_runtime.handle();
        #[cfg(feature = "stylos")]
        let network = network_runtime.handle();
        let background = background_runtime.as_ref().map(OwnedRuntime::handle);

        Ok(Self {
            _tui_runtime: tui_runtime,
            _core_runtime: core_runtime,
            _network_runtime: network_runtime,
            _background_runtime: background_runtime,
            tui,
            core,
            #[cfg(feature = "stylos")]
            network,
            background,
        })
    }

    pub fn tui(&self) -> Option<DomainHandle> {
        self.tui.clone()
    }

    pub fn core(&self) -> DomainHandle {
        self.core.clone()
    }

    #[cfg(feature = "stylos")]
    pub fn network(&self) -> DomainHandle {
        self.network.clone()
    }

    pub fn background(&self) -> Option<DomainHandle> {
        self.background.clone()
    }
}

fn build_multi_thread_runtime(
    domain: RuntimeDomain,
    worker_threads: usize,
) -> Result<OwnedRuntime> {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name(format!("themion-{}", domain.as_str()))
        .build()?;
    Ok(OwnedRuntime {
        name: domain,
        runtime,
    })
}


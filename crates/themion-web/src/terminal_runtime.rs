use anyhow::{anyhow, bail, Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

const TERMINAL_SCROLLBACK_LIMIT_BYTES: usize = 262_144;
const DEFAULT_TERMINAL_COLS: u16 = 120;
const DEFAULT_TERMINAL_ROWS: u16 = 40;

#[derive(Clone)]
pub struct TerminalService {
    request_tx: mpsc::Sender<TerminalRequest>,
}

enum TerminalRequest {
    CreateTerminal {
        response_tx: oneshot::Sender<Result<TerminalDescriptor>>,
    },
    ListTerminals {
        response_tx: oneshot::Sender<Result<Vec<TerminalDescriptor>>>,
    },
    AttachTerminal {
        terminal_id: u64,
        response_tx: oneshot::Sender<Result<TerminalAttachHandle>>,
    },
    Input {
        terminal_id: u64,
        data: Vec<u8>,
        response_tx: oneshot::Sender<Result<()>>,
    },
    Resize {
        terminal_id: u64,
        cols: u16,
        rows: u16,
        response_tx: oneshot::Sender<Result<()>>,
    },
    CloseTerminal {
        terminal_id: u64,
        response_tx: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalDescriptor {
    pub terminal_id: u64,
    pub label: String,
}

pub struct TerminalAttachHandle {
    pub descriptor: TerminalDescriptor,
    pub scrollback: String,
    pub output_rx: tokio_mpsc::UnboundedReceiver<String>,
}

struct TerminalRegistry {
    next_terminal_id: AtomicU64,
    shell: String,
    cwd: Option<String>,
    terminals: Mutex<HashMap<u64, TerminalEntry>>,
}

struct TerminalEntry {
    descriptor: TerminalDescriptor,
    input_tx: tokio_mpsc::UnboundedSender<Vec<u8>>,
    resize_tx: tokio_mpsc::UnboundedSender<TerminalResize>,
    subscribers: Vec<tokio_mpsc::UnboundedSender<String>>,
    scrollback: String,
    _child: Box<dyn Child + Send + Sync>,
}

#[derive(Clone, Copy)]
struct TerminalResize {
    cols: u16,
    rows: u16,
}

pub fn spawn_background_service_runtime(
    ready_tx: oneshot::Sender<Result<TerminalService>>,
) -> Result<thread::JoinHandle<Result<()>>> {
    thread::Builder::new()
        .name("themion-web-background".to_string())
        .spawn(move || {
            let runtime = build_background_runtime()?;
            runtime.block_on(run_background_services(ready_tx))
        })
        .context("failed to spawn background service runtime thread")
}

fn build_background_runtime() -> Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("themion-web-background")
        .build()
        .context("failed to build background service runtime")
}

async fn run_background_services(
    ready_tx: oneshot::Sender<Result<TerminalService>>,
) -> Result<()> {
    let registry = Arc::new(TerminalRegistry::new()?);
    let (request_tx, request_rx) = mpsc::channel::<TerminalRequest>();
    let service = TerminalService { request_tx };
    let _ = ready_tx.send(Ok(service));
    process_terminal_requests(registry, request_rx).await
}

async fn process_terminal_requests(
    registry: Arc<TerminalRegistry>,
    request_rx: mpsc::Receiver<TerminalRequest>,
) -> Result<()> {
    while let Ok(request) = request_rx.recv() {
        match request {
            TerminalRequest::CreateTerminal { response_tx } => {
                let _ = response_tx.send(registry.create_terminal());
            }
            TerminalRequest::ListTerminals { response_tx } => {
                let _ = response_tx.send(registry.list_terminals());
            }
            TerminalRequest::AttachTerminal {
                terminal_id,
                response_tx,
            } => {
                let _ = response_tx.send(registry.attach_terminal(terminal_id));
            }
            TerminalRequest::Input {
                terminal_id,
                data,
                response_tx,
            } => {
                let _ = response_tx.send(registry.send_input(terminal_id, data));
            }
            TerminalRequest::Resize {
                terminal_id,
                cols,
                rows,
                response_tx,
            } => {
                let _ = response_tx.send(registry.resize_terminal(terminal_id, cols, rows));
            }
            TerminalRequest::CloseTerminal {
                terminal_id,
                response_tx,
            } => {
                let _ = response_tx.send(registry.close_terminal(terminal_id));
            }
        }
    }

    bail!("terminal service request channel closed")
}

impl TerminalRegistry {
    fn new() -> Result<Self> {
        Ok(Self {
            next_terminal_id: AtomicU64::new(1),
            shell: resolve_shell(),
            cwd: env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(|value| value.to_string())),
            terminals: Mutex::new(HashMap::new()),
        })
    }

    fn create_terminal(self: &Arc<Self>) -> Result<TerminalDescriptor> {
        let terminal_id = self.next_terminal_id.fetch_add(1, Ordering::Relaxed);
        let descriptor = TerminalDescriptor {
            terminal_id,
            label: format!("Shell {terminal_id}"),
        };

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_TERMINAL_ROWS,
                cols: DEFAULT_TERMINAL_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open pty")?;

        let mut cmd = CommandBuilder::new(&self.shell);
        if let Some(cwd) = self.cwd.as_deref() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn shell '{}'", self.shell))?;

        let writer = pair.master.take_writer().context("failed to get pty writer")?;
        let reader = pair.master.try_clone_reader().context("failed to clone pty reader")?;
        let resizer = pair.master;

        let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
        let (output_tx, output_rx) = tokio_mpsc::unbounded_channel::<String>();
        let (resize_tx, resize_rx) = tokio_mpsc::unbounded_channel::<TerminalResize>();

        spawn_terminal_input_loop(writer, input_rx);
        spawn_terminal_output_loop(reader, output_tx);
        spawn_terminal_resize_loop(resizer, resize_rx);
        spawn_terminal_broadcast_loop(terminal_id, Arc::clone(self), output_rx);

        self.terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?
            .insert(
                terminal_id,
                TerminalEntry {
                    descriptor: descriptor.clone(),
                    input_tx,
                    resize_tx,
                    subscribers: Vec::new(),
                    scrollback: String::new(),
                    _child: child,
                },
            );

        Ok(descriptor)
    }

    fn list_terminals(&self) -> Result<Vec<TerminalDescriptor>> {
        let mut terminals: Vec<_> = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect();
        terminals.sort_by_key(|terminal| terminal.terminal_id);
        Ok(terminals)
    }

    fn attach_terminal(&self, terminal_id: u64) -> Result<TerminalAttachHandle> {
        let (subscriber_tx, subscriber_rx) = tokio_mpsc::unbounded_channel::<String>();
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry.subscribers.push(subscriber_tx);
        Ok(TerminalAttachHandle {
            descriptor: entry.descriptor.clone(),
            scrollback: entry.scrollback.clone(),
            output_rx: subscriber_rx,
        })
    }

    fn send_input(&self, terminal_id: u64, data: Vec<u8>) -> Result<()> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry
            .input_tx
            .send(data)
            .map_err(|_| anyhow!("terminal input channel closed"))
    }

    fn resize_terminal(&self, terminal_id: u64, cols: u16, rows: u16) -> Result<()> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry
            .resize_tx
            .send(TerminalResize { cols, rows })
            .map_err(|_| anyhow!("terminal resize channel closed"))
    }

    fn close_terminal(&self, terminal_id: u64) -> Result<()> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        terminals
            .remove(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        Ok(())
    }

    fn fan_out_output(&self, terminal_id: u64, data: String) -> Result<()> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let Some(entry) = terminals.get_mut(&terminal_id) else {
            return Ok(());
        };

        entry.scrollback.push_str(&data);
        trim_scrollback(&mut entry.scrollback);
        entry
            .subscribers
            .retain(|subscriber| subscriber.send(data.clone()).is_ok());
        Ok(())
    }
}

fn trim_scrollback(scrollback: &mut String) {
    if scrollback.len() <= TERMINAL_SCROLLBACK_LIMIT_BYTES {
        return;
    }
    let drop_bytes = scrollback.len() - TERMINAL_SCROLLBACK_LIMIT_BYTES;
    let drop_at = scrollback
        .char_indices()
        .find_map(|(index, _)| (index >= drop_bytes).then_some(index))
        .unwrap_or(scrollback.len());
    scrollback.drain(..drop_at);
}

fn spawn_terminal_input_loop(
    mut writer: Box<dyn Write + Send>,
    mut input_rx: tokio_mpsc::UnboundedReceiver<Vec<u8>>,
) {
    tokio::task::spawn_blocking(move || {
        while let Some(bytes) = input_rx.blocking_recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
        }
    });
}

fn spawn_terminal_output_loop(
    mut reader: Box<dyn Read + Send>,
    output_tx: tokio_mpsc::UnboundedSender<String>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0_u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(count) => {
                    let text = String::from_utf8_lossy(&buf[..count]).to_string();
                    if output_tx.send(text).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_terminal_resize_loop(
    master: Box<dyn MasterPty + Send>,
    mut resize_rx: tokio_mpsc::UnboundedReceiver<TerminalResize>,
) {
    tokio::spawn(async move {
        while let Some(resize) = resize_rx.recv().await {
            let _ = master.resize(PtySize {
                rows: resize.rows,
                cols: resize.cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    });
}

fn spawn_terminal_broadcast_loop(
    terminal_id: u64,
    registry: Arc<TerminalRegistry>,
    mut output_rx: tokio_mpsc::UnboundedReceiver<String>,
) {
    tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            let _ = registry.fan_out_output(terminal_id, data);
        }
    });
}

fn resolve_shell() -> String {
    env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

impl TerminalService {
    pub async fn create_terminal(&self) -> Result<TerminalDescriptor> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::CreateTerminal { response_tx })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped create response")?
    }

    pub async fn list_terminals(&self) -> Result<Vec<TerminalDescriptor>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::ListTerminals { response_tx })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped list response")?
    }

    pub async fn attach_terminal(&self, terminal_id: u64) -> Result<TerminalAttachHandle> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::AttachTerminal {
                terminal_id,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped attach response")?
    }

    pub async fn send_input(&self, terminal_id: u64, data: Vec<u8>) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::Input {
                terminal_id,
                data,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped input response")?
    }

    pub async fn resize_terminal(&self, terminal_id: u64, cols: u16, rows: u16) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::Resize {
                terminal_id,
                cols,
                rows,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped resize response")?
    }

    pub async fn close_terminal(&self, terminal_id: u64) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::CloseTerminal {
                terminal_id,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped close response")?
    }
}

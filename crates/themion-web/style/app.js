const panels = () => document.querySelectorAll('[data-tab-panel]');
const tabs = () => document.querySelectorAll('[data-tab-target]');
const TERMINAL_ENDPOINT = '/api/terminal/ws';

let terminalController = null;

function selectTab(tabId) {
  panels().forEach((panel) => {
    const active = panel.dataset.tabPanel === tabId;
    panel.hidden = !active;
  });

  tabs().forEach((tab) => {
    const active = tab.dataset.tabTarget === tabId;
    tab.classList.toggle('is-active', active);
    tab.setAttribute('aria-selected', String(active));
  });
}

function setupTabs() {
  tabs().forEach((tab) => {
    tab.addEventListener('click', () => selectTab(tab.dataset.tabTarget));
  });
}

function setupSidebarToggle() {
  const workspace = document.getElementById('workspace');
  const button = document.getElementById('sidebar-toggle');
  if (!workspace || !button) return;

  button.addEventListener('click', () => {
    const collapsed = workspace.classList.toggle('sidebar-collapsed');
    button.setAttribute('aria-pressed', String(collapsed));
  });
}

function setTerminalStatus(state, message) {
  const status = document.getElementById('terminal-status');
  if (!status) return;
  status.dataset.state = state;
  status.textContent = message;
}

function measureTerminalSize(root) {
  const width = root.clientWidth || 960;
  const height = root.clientHeight || 480;
  const cols = Math.max(20, Math.floor(width / 9));
  const rows = Math.max(8, Math.floor(height / 18));
  return { cols, rows };
}

function connectTerminal() {
  const root = document.getElementById('terminal-root');
  const reconnect = document.getElementById('terminal-reconnect');
  if (!root || !window.Terminal) return;

  if (terminalController) {
    terminalController.dispose();
    terminalController = null;
  }

  root.innerHTML = '';
  setTerminalStatus('connecting', 'Connecting terminal…');

  const terminal = new window.Terminal({
    convertEol: true,
    cursorBlink: true,
    fontFamily: 'JetBrains Mono, SFMono-Regular, ui-monospace, monospace',
    fontSize: 13,
    theme: {
      background: '#0b1016',
      foreground: '#f3f6fb',
      cursor: '#f3f6fb',
      selectionBackground: '#334155',
    },
  });

  terminal.open(root);

  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(`${protocol}//${window.location.host}${TERMINAL_ENDPOINT}`);

  const sendResize = () => {
    if (socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'resize', ...measureTerminalSize(root) }));
  };

  socket.addEventListener('open', () => {
    setTerminalStatus('connected', 'Terminal connected');
    terminal.focus();
    sendResize();
  });

  socket.addEventListener('message', (event) => {
    if (typeof event.data === 'string') {
      terminal.write(event.data);
    }
  });

  socket.addEventListener('close', () => {
    setTerminalStatus('idle', 'Terminal disconnected');
  });

  socket.addEventListener('error', () => {
    setTerminalStatus('error', 'Terminal connection error');
  });

  terminal.onData((data) => {
    if (socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'input', data }));
  });

  const resizeObserver = new ResizeObserver(() => sendResize());
  resizeObserver.observe(root);
  window.addEventListener('resize', sendResize);

  reconnect?.addEventListener('click', () => connectTerminal(), { once: true });

  terminalController = {
    dispose() {
      resizeObserver.disconnect();
      window.removeEventListener('resize', sendResize);
      socket.close();
      terminal.dispose();
    },
  };
}

function initAppShell() {
  setupTabs();
  setupSidebarToggle();
  selectTab('tab-main');
  connectTerminal();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initAppShell, { once: true });
} else {
  initAppShell();
}

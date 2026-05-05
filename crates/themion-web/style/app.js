const panels = () => document.querySelectorAll('[data-tab-panel]');
const tabs = () => document.querySelectorAll('[data-tab-target]');
const TERMINAL_ENDPOINT = '/api/terminal/ws';
const STORAGE_KEY = 'themion-web.terminals.v1';

let terminalManager = null;

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
  return {
    cols: Math.max(20, Math.floor(width / 9)),
    rows: Math.max(8, Math.floor(height / 18)),
  };
}

function loadTerminalState() {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { openTerminalIds: [], activeTerminalId: null };
    const parsed = JSON.parse(raw);
    return {
      openTerminalIds: Array.isArray(parsed.openTerminalIds) ? parsed.openTerminalIds : [],
      activeTerminalId: typeof parsed.activeTerminalId === 'number' ? parsed.activeTerminalId : null,
    };
  } catch {
    return { openTerminalIds: [], activeTerminalId: null };
  }
}

function saveTerminalState() {
  if (!terminalManager) return;
  const payload = {
    openTerminalIds: Array.from(terminalManager.sessions.keys()),
    activeTerminalId: terminalManager.activeId,
  };
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
}

function makeTerminal() {
  return new window.Terminal({
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
}

function createSessionDom(terminal) {
  const tabButton = document.createElement('button');
  tabButton.className = 'terminal-session-tab';
  tabButton.type = 'button';
  tabButton.setAttribute('role', 'tab');

  const labelSpan = document.createElement('span');
  labelSpan.className = 'terminal-session-label';
  labelSpan.textContent = terminal.label;

  const closeButton = document.createElement('button');
  closeButton.className = 'terminal-session-close';
  closeButton.type = 'button';
  closeButton.setAttribute('aria-label', `Close ${terminal.label}`);
  closeButton.textContent = '×';

  tabButton.append(labelSpan, closeButton);

  const panel = document.createElement('section');
  panel.className = 'terminal-panel';
  panel.hidden = true;

  const root = document.createElement('div');
  root.className = 'terminal-root';
  panel.append(root);

  return { tabButton, labelSpan, closeButton, panel, root };
}

function ensureSession(terminal) {
  if (!terminalManager) return null;
  const existing = terminalManager.sessions.get(terminal.terminal_id);
  if (existing) {
    existing.label = terminal.label;
    existing.labelSpan.textContent = terminal.label;
    existing.closeButton.setAttribute('aria-label', `Close ${terminal.label}`);
    return existing;
  }

  const dom = createSessionDom(terminal);
  const session = {
    id: terminal.terminal_id,
    label: terminal.label,
    attached: false,
    attaching: false,
    state: 'connecting',
    tabButton: dom.tabButton,
    labelSpan: dom.labelSpan,
    closeButton: dom.closeButton,
    panel: dom.panel,
    root: dom.root,
    terminal: makeTerminal(),
    resizeObserver: null,
  };

  session.terminal.open(session.root);
  session.terminal.onData((data) => {
    if (!terminalManager?.socketOpen) return;
    sendSocketMessage({ type: 'input', terminal_id: session.id, data });
  });

  session.resizeObserver = new ResizeObserver(() => {
    if (!terminalManager?.socketOpen) return;
    sendSocketMessage({ type: 'resize', terminal_id: session.id, ...measureTerminalSize(session.root) });
  });
  session.resizeObserver.observe(session.root);

  dom.closeButton.addEventListener('click', (event) => {
    event.stopPropagation();
    closeSession(session.id, true);
  });
  dom.tabButton.addEventListener('click', () => activateSession(session.id));

  terminalManager.tabsRoot.append(dom.tabButton);
  terminalManager.panelsRoot.append(dom.panel);
  terminalManager.sessions.set(session.id, session);
  saveTerminalState();
  return session;
}

function activateSession(sessionId) {
  if (!terminalManager) return;
  const session = terminalManager.sessions.get(sessionId);
  if (!session) return;

  terminalManager.activeId = sessionId;
  terminalManager.sessions.forEach((entry) => {
    const active = entry.id === sessionId;
    entry.tabButton.classList.toggle('is-active', active);
    entry.panel.classList.toggle('is-active', active);
    entry.tabButton.setAttribute('aria-selected', String(active));
    entry.panel.hidden = !active;
  });

  setTerminalStatus(terminalManager.socketOpen ? 'connected' : 'connecting', `${session.label} ${terminalManager.socketOpen ? 'connected' : 'reconnecting'}`);
  session.terminal.focus();
  if (terminalManager.socketOpen) {
    sendSocketMessage({ type: 'resize', terminal_id: session.id, ...measureTerminalSize(session.root) });
  }
  saveTerminalState();
}

function removeSession(sessionId) {
  const session = terminalManager?.sessions.get(sessionId);
  if (!session) return;
  session.resizeObserver?.disconnect();
  session.terminal.dispose();
  session.tabButton.remove();
  session.panel.remove();
  terminalManager.sessions.delete(sessionId);
}

function closeSession(sessionId, notifyServer) {
  if (!terminalManager) return;
  if (notifyServer && terminalManager.socketOpen) {
    sendSocketMessage({ type: 'close_terminal', terminal_id: sessionId });
  }

  removeSession(sessionId);

  if (!terminalManager.sessions.size) {
    terminalManager.activeId = null;
    setTerminalStatus(terminalManager.socketOpen ? 'connected' : 'idle', terminalManager.socketOpen ? 'No open terminals' : 'Disconnected');
    saveTerminalState();
    return;
  }

  const nextId = terminalManager.sessions.keys().next().value;
  activateSession(nextId);
}

function sendSocketMessage(message) {
  if (!terminalManager?.socket || terminalManager.socket.readyState !== WebSocket.OPEN) return;
  terminalManager.socket.send(JSON.stringify(message));
}

function attachSession(sessionId) {
  const session = terminalManager?.sessions.get(sessionId);
  if (!session || session.attached || session.attaching || !terminalManager.socketOpen) return;
  session.attaching = true;
  sendSocketMessage({ type: 'attach_terminal', terminal_id: sessionId });
}

function createNewTerminal() {
  sendSocketMessage({ type: 'create_terminal' });
}

function restoreSessions(terminals) {
  if (!terminalManager) return;
  const saved = loadTerminalState();
  const wantedIds = saved.openTerminalIds.filter((id) => terminals.some((terminal) => terminal.terminal_id === id));

  terminals.forEach((terminal) => {
    if (wantedIds.includes(terminal.terminal_id)) {
      ensureSession(terminal);
    }
  });

  if (!terminalManager.sessions.size) {
    if (terminals.length) {
      const session = ensureSession(terminals[0]);
      attachSession(session.id);
      activateSession(session.id);
    } else {
      createNewTerminal();
    }
    return;
  }

  terminalManager.sessions.forEach((session) => attachSession(session.id));
  const activeId = wantedIds.includes(saved.activeTerminalId) ? saved.activeTerminalId : terminalManager.sessions.keys().next().value;
  activateSession(activeId);
}

function connectSocket() {
  if (!terminalManager) return;
  terminalManager.socket?.close();
  terminalManager.socketOpen = false;
  terminalManager.sessions.forEach((session) => {
    session.attached = false;
    session.attaching = false;
    session.state = 'connecting';
  });

  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(`${protocol}//${window.location.host}${TERMINAL_ENDPOINT}`);
  terminalManager.socket = socket;
  setTerminalStatus('connecting', 'Connecting terminal service…');

  socket.addEventListener('open', () => {
    terminalManager.socketOpen = true;
    setTerminalStatus('connected', terminalManager.activeId ? `${terminalManager.sessions.get(terminalManager.activeId)?.label ?? 'Terminal'} connected` : 'Connected');
    sendSocketMessage({ type: 'list_terminals' });
  });

  socket.addEventListener('message', (event) => {
    if (typeof event.data !== 'string') return;
    const message = JSON.parse(event.data);
    handleSocketMessage(message);
  });

  socket.addEventListener('close', () => {
    terminalManager.socketOpen = false;
    terminalManager.sessions.forEach((session) => {
      session.attached = false;
      session.attaching = false;
      session.state = 'connecting';
    });
    setTerminalStatus('idle', 'Socket disconnected');
  });

  socket.addEventListener('error', () => {
    terminalManager.socketOpen = false;
    setTerminalStatus('error', 'Socket error');
  });
}

function handleSocketMessage(message) {
  if (!terminalManager) return;

  switch (message.type) {
    case 'terminal_list': {
      restoreSessions(message.terminals || []);
      break;
    }
    case 'terminal_created': {
      const session = ensureSession(message.terminal);
      activateSession(session.id);
      break;
    }
    case 'terminal_attached': {
      const session = ensureSession(message.terminal);
      session.attached = true;
      session.attaching = false;
      session.state = 'connected';
      session.terminal.reset();
      if (message.scrollback) {
        session.terminal.write(message.scrollback);
      }
      sendSocketMessage({ type: 'resize', terminal_id: session.id, ...measureTerminalSize(session.root) });
      if (terminalManager.activeId === session.id) {
        setTerminalStatus('connected', `${session.label} connected`);
        session.terminal.focus();
      }
      saveTerminalState();
      break;
    }
    case 'terminal_output': {
      const session = terminalManager.sessions.get(message.terminal_id);
      if (session) {
        session.terminal.write(message.data);
      }
      break;
    }
    case 'terminal_closed': {
      closeSession(message.terminal_id, false);
      break;
    }
    case 'error': {
      setTerminalStatus('error', message.message || 'Terminal error');
      break;
    }
  }
}

function initTerminalManager() {
  const tabsRoot = document.getElementById('terminal-tabs');
  const panelsRoot = document.getElementById('terminal-panels');
  const newButton = document.getElementById('terminal-new');
  const reconnectButton = document.getElementById('terminal-reconnect');
  if (!tabsRoot || !panelsRoot || !newButton || !reconnectButton || !window.Terminal) return;

  terminalManager = {
    tabsRoot,
    panelsRoot,
    sessions: new Map(),
    activeId: null,
    socket: null,
    socketOpen: false,
  };

  newButton.addEventListener('click', () => createNewTerminal());
  reconnectButton.addEventListener('click', () => connectSocket());
  window.addEventListener('beforeunload', () => saveTerminalState());

  connectSocket();
}

function initAppShell() {
  setupTabs();
  setupSidebarToggle();
  selectTab('tab-main');
  initTerminalManager();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initAppShell, { once: true });
} else {
  initAppShell();
}

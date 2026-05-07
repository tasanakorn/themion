const panels = () => document.querySelectorAll('[data-tab-panel]');
const tabs = () => document.querySelectorAll('[data-tab-target]');
const SHARED_WS_ENDPOINT = '/api/ws';
const TERMINAL_STORAGE_KEY = 'themion-web.terminals.v1';
const DEFAULT_TERMINAL_COLS = 120;
const DEFAULT_TERMINAL_ROWS = 40;

let terminalManager = null;
let agentManager = null;
let sharedSocketManager = null;

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

function sharedSocketUrl() {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}${SHARED_WS_ENDPOINT}`;
}

function initSharedSocketManager() {
  sharedSocketManager = {
    socket: null,
    open: false,
  };
}

function sendSharedMessage(message) {
  if (!sharedSocketManager?.socket || sharedSocketManager.socket.readyState !== WebSocket.OPEN) {
    return false;
  }
  sharedSocketManager.socket.send(JSON.stringify(message));
  return true;
}

function connectSharedSocket() {
  if (!sharedSocketManager) return;
  sharedSocketManager.socket?.close();
  sharedSocketManager.open = false;

  if (terminalManager) {
    terminalManager.socketOpen = false;
    terminalManager.restoredInitialSessions = false;
    terminalManager.sessions.forEach((session) => {
      session.attached = false;
      session.attaching = false;
      session.state = 'connecting';
    });
    setTerminalStatus('connecting', 'Connecting shared websocket…');
  }

  if (agentManager) {
    agentManager.socketOpen = false;
    agentManager.attachedAgents.clear();
    setAgentStatus('connecting', 'Connecting shared websocket…');
  }

  const socket = new WebSocket(sharedSocketUrl());
  sharedSocketManager.socket = socket;

  socket.addEventListener('open', () => {
    sharedSocketManager.open = true;

    if (terminalManager) {
      terminalManager.socketOpen = true;
      setTerminalStatus('connected', 'Connected');
      sendTerminalMessage({ type: 'list_terminals' });
    }

    if (agentManager) {
      agentManager.socketOpen = true;
      setAgentStatus('connected', 'Connected to web agent runtime');
      sendAgentMessage({ type: 'snapshot' });
      if (agentManager.selectedAgentId) {
        attachSelectedAgent(agentManager.selectedAgentId);
      }
    }
  });

  socket.addEventListener('message', (event) => {
    if (typeof event.data !== 'string') return;
    const message = JSON.parse(event.data);
    if (message.domain === 'terminal') {
      handleTerminalSocketMessage(message);
    } else if (message.domain === 'agent') {
      handleAgentSocketMessage(message);
    }
  });

  socket.addEventListener('close', () => {
    sharedSocketManager.open = false;
    if (terminalManager) {
      terminalManager.socketOpen = false;
      terminalManager.restoredInitialSessions = false;
      terminalManager.sessions.forEach((session) => {
        session.attached = false;
        session.attaching = false;
        session.state = 'connecting';
      });
      setTerminalStatus('idle', 'Shared websocket disconnected');
    }
    if (agentManager) {
      agentManager.socketOpen = false;
      agentManager.attachedAgents.clear();
      if (agentManager.submitEl) agentManager.submitEl.disabled = true;
      if (agentManager.composerEl) agentManager.composerEl.disabled = true;
      setAgentStatus('idle', 'Shared websocket disconnected');
    }
  });

  socket.addEventListener('error', () => {
    sharedSocketManager.open = false;
    if (terminalManager) {
      terminalManager.socketOpen = false;
      setTerminalStatus('error', 'Shared websocket error');
    }
    if (agentManager) {
      agentManager.socketOpen = false;
      setAgentStatus('error', 'Shared websocket error');
    }
  });
}

function setTerminalStatus(state, message) {
  const status = document.getElementById('terminal-status');
  if (!status) return;
  status.dataset.state = state;
  status.textContent = message;
}

function fitTerminal(session) {
  if (!terminalManager?.socketOpen || !session) return;
  const nextSize = {
    cols: Math.max(20, session.terminal.cols || DEFAULT_TERMINAL_COLS),
    rows: Math.max(8, session.terminal.rows || DEFAULT_TERMINAL_ROWS),
  };
  sendTerminalMessage({ type: 'resize', terminal_id: session.id, ...nextSize });
}

function loadTerminalState() {
  try {
    const raw = window.localStorage.getItem(TERMINAL_STORAGE_KEY);
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
  window.localStorage.setItem(TERMINAL_STORAGE_KEY, JSON.stringify(payload));
}

function makeTerminal() {
  return new window.Terminal({
    cols: DEFAULT_TERMINAL_COLS,
    rows: DEFAULT_TERMINAL_ROWS,
    convertEol: true,
    cursorBlink: true,
    fontFamily: 'JetBrains Mono Nerd Font, JetBrains Mono, SFMono-Regular, ui-monospace, monospace',
    fontSize: 14,
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
    if (!terminalManager?.socketOpen || !session.attached) return;
    sendTerminalMessage({ type: 'input', terminal_id: session.id, data });
  });

  session.resizeObserver = new ResizeObserver(() => {
    fitTerminal(session);
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

  setTerminalStatus(
    terminalManager.socketOpen ? 'connected' : 'connecting',
    `${session.label} ${terminalManager.socketOpen ? 'connected' : 'reconnecting'}`,
  );
  session.terminal.focus();
  fitTerminal(session);
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
    sendTerminalMessage({ type: 'close_terminal', terminal_id: sessionId });
  }

  removeSession(sessionId);

  if (!terminalManager.sessions.size) {
    terminalManager.activeId = null;
    setTerminalStatus(
      terminalManager.socketOpen ? 'connected' : 'idle',
      terminalManager.socketOpen ? 'No open terminals' : 'Disconnected',
    );
    saveTerminalState();
    return;
  }

  const nextId = terminalManager.sessions.keys().next().value;
  activateSession(nextId);
}

function sendTerminalMessage(message) {
  return sendSharedMessage({ domain: 'terminal', ...message });
}

function attachSession(sessionId) {
  const session = terminalManager?.sessions.get(sessionId);
  if (!session || session.attached || session.attaching || !terminalManager.socketOpen) return;
  session.attaching = true;
  sendTerminalMessage({ type: 'attach_terminal', terminal_id: sessionId });
}

function createNewTerminal() {
  sendTerminalMessage({ type: 'create_terminal' });
}

function restoreSessions(terminals) {
  if (!terminalManager) return;
  const saved = loadTerminalState();
  const wantedIds = saved.openTerminalIds.filter((id) =>
    terminals.some((terminal) => terminal.terminal_id === id),
  );

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
  const activeId = wantedIds.includes(saved.activeTerminalId)
    ? saved.activeTerminalId
    : terminalManager.sessions.keys().next().value;
  activateSession(activeId);
}

function handleTerminalSocketMessage(message) {
  if (!terminalManager || message.domain !== 'terminal') return;

  switch (message.type) {
    case 'terminal_list': {
      if (!terminalManager.restoredInitialSessions) {
        terminalManager.restoredInitialSessions = true;
        restoreSessions(message.terminals || []);
      }
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
      fitTerminal(session);
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
    socketOpen: false,
    restoredInitialSessions: false,
  };

  newButton.addEventListener('click', () => createNewTerminal());
  reconnectButton.addEventListener('click', () => connectSharedSocket());
  window.addEventListener('beforeunload', () => saveTerminalState());
}

function setAgentStatus(state, message) {
  const status = agentManager?.statusEl;
  if (!status) return;
  status.dataset.state = state;
  status.textContent = message;
}

function sendAgentMessage(message) {
  return sendSharedMessage({ domain: 'agent', ...message });
}

function attachSelectedAgent(agentId) {
  if (!agentManager?.socketOpen || !agentId || agentManager.attachedAgents.has(agentId)) {
    return;
  }
  if (sendAgentMessage({ type: 'attach', agent_id: agentId })) {
    agentManager.attachedAgents.add(agentId);
  }
}

function renderAgentRoster() {
  if (!agentManager?.rosterEl) return;
  agentManager.rosterEl.innerHTML = '';
  const agents = Array.from(agentManager.agents.values());
  agents.sort((a, b) => a.agent_id.localeCompare(b.agent_id));

  if (!agents.length) {
    const empty = document.createElement('div');
    empty.className = 'agent-empty';
    empty.textContent = 'No web agents available.';
    agentManager.rosterEl.append(empty);
    return;
  }

  agents.forEach((agent) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'agent-roster-item';
    if (agentManager.selectedAgentId === agent.agent_id) {
      button.classList.add('is-active');
    }

    const top = document.createElement('div');
    top.className = 'agent-roster-top';

    const label = document.createElement('span');
    label.className = 'agent-roster-label';
    label.textContent = agent.label;

    const status = document.createElement('span');
    status.className = 'agent-roster-status';
    status.dataset.state = agent.busy ? 'busy' : agent.status || 'idle';
    status.textContent = agent.busy ? 'busy' : agent.status || 'idle';

    top.append(label, status);

    const meta = document.createElement('div');
    meta.className = 'agent-roster-meta';
    meta.textContent = `${agent.agent_id} · ${agent.roles.join(', ')} · ${agent.provider} · ${agent.model}`;

    button.append(top, meta);

    if (agent.warning) {
      const warning = document.createElement('div');
      warning.className = 'agent-warning';
      warning.textContent = agent.warning;
      button.append(warning);
    }
    button.addEventListener('click', () => selectAgent(agent.agent_id));
    agentManager.rosterEl.append(button);
  });
}

function renderAgentTranscript() {
  if (!agentManager?.transcriptEl) return;
  agentManager.transcriptEl.innerHTML = '';
  const agent = agentManager.selectedAgentId
    ? agentManager.agents.get(agentManager.selectedAgentId)
    : null;

  if (!agent) {
    const empty = document.createElement('div');
    empty.className = 'agent-empty';
    empty.textContent = 'Select an agent to view its transcript.';
    agentManager.transcriptEl.append(empty);
    if (agentManager.deleteEl) {
      agentManager.deleteEl.disabled = true;
    }
    return;
  }

  if (!agent.transcript?.length) {
    const empty = document.createElement('div');
    empty.className = 'agent-empty';
    empty.textContent = 'No turns yet for this agent.';
    agentManager.transcriptEl.append(empty);
  } else {
    agent.transcript.forEach((entry) => {
      const row = document.createElement('div');
      row.className = `agent-transcript-row is-${entry.kind}`;

      const kind = document.createElement('div');
      kind.className = 'agent-transcript-kind';
      kind.textContent = entry.kind;

      const text = document.createElement('pre');
      text.className = 'agent-transcript-text';
      text.textContent = entry.text;

      row.append(kind, text);
      agentManager.transcriptEl.append(row);
    });
  }

  if (agentManager.detailsEl) {
    const warning = agent.warning ? `\nwarning: ${agent.warning}` : '';
    agentManager.detailsEl.textContent = `${agent.agent_id} · ${agent.roles.join(', ')} · ${agent.provider} · ${agent.model}${warning}`;
  }
  if (agentManager.titleEl) {
    agentManager.titleEl.textContent = agent.label;
  }
  if (agentManager.composerEl) {
    agentManager.composerEl.disabled = !agentManager.socketOpen || agent.busy;
  }
  if (agentManager.submitEl) {
    agentManager.submitEl.disabled = !agentManager.socketOpen || agent.busy;
  }
  if (agentManager.deleteEl) {
    agentManager.deleteEl.disabled = !agentManager.socketOpen || agent.agent_id === 'master';
  }
  agentManager.transcriptEl.scrollTop = agentManager.transcriptEl.scrollHeight;
}

function selectAgent(agentId) {
  if (!agentManager) return;
  agentManager.selectedAgentId = agentId;
  attachSelectedAgent(agentId);
  renderAgentRoster();
  renderAgentTranscript();
  if (agentManager.composerEl && !agentManager.composerEl.disabled) {
    agentManager.composerEl.focus();
  }
}

function applyAgentSnapshot(message) {
  const existing = agentManager.agents.get(message.agent_id) || {};
  agentManager.agents.set(message.agent_id, {
    ...existing,
    agent_id: message.agent_id,
    label: message.label,
    roles: message.roles || [],
    busy: Boolean(message.busy),
    provider: message.provider || existing.provider || '',
    model: message.model || existing.model || '',
    transcript: message.transcript || [],
    status: message.status || existing.status || 'idle',
    warning: message.warning || existing.warning || null,
  });
  agentManager.attachedAgents.add(message.agent_id);
  if (!agentManager.selectedAgentId) {
    agentManager.selectedAgentId = message.agent_id;
  }
  renderAgentRoster();
  renderAgentTranscript();
}

function applyAgentTranscriptDelta(message) {
  const agent = agentManager.agents.get(message.agent_id);
  if (!agent) return;
  agent.transcript = Array.isArray(agent.transcript) ? agent.transcript : [];
  if (message.replace_last && agent.transcript.length) {
    const last = agent.transcript[agent.transcript.length - 1];
    if (last.kind === message.kind) {
      last.text += message.text;
    } else {
      agent.transcript.push({ kind: message.kind, text: message.text });
    }
  } else {
    agent.transcript.push({ kind: message.kind, text: message.text });
  }
  renderAgentTranscript();
}

function handleAgentSocketMessage(message) {
  if (!agentManager || message.domain !== 'agent') return;

  switch (message.type) {
    case 'roster_snapshot': {
      const previous = new Map(agentManager.agents);
      agentManager.agents.clear();
      (message.agents || []).forEach((agent) => {
        const prior = previous.get(agent.agent_id);
        agentManager.agents.set(agent.agent_id, {
          ...agent,
          warning: agent.warning || null,
          transcript: prior?.transcript || [],
        });
      });
      if (!agentManager.selectedAgentId && message.agents?.length) {
        agentManager.selectedAgentId = message.agents[0].agent_id;
      }
      if (agentManager.selectedAgentId && !agentManager.agents.has(agentManager.selectedAgentId)) {
        agentManager.selectedAgentId = message.agents?.[0]?.agent_id || null;
      }
      renderAgentRoster();
      renderAgentTranscript();
      if (agentManager.selectedAgentId) {
        attachSelectedAgent(agentManager.selectedAgentId);
      }
      break;
    }
    case 'agent_snapshot': {
      applyAgentSnapshot(message);
      break;
    }
    case 'busy_state': {
      const agent = agentManager.agents.get(message.agent_id);
      if (agent) {
        agent.busy = Boolean(message.busy);
        agent.status = message.busy ? 'running' : 'idle';
        renderAgentRoster();
        renderAgentTranscript();
      }
      break;
    }
    case 'transcript_delta': {
      applyAgentTranscriptDelta(message);
      break;
    }
    case 'completed': {
      const agent = agentManager.agents.get(message.agent_id);
      if (agent) {
        agent.busy = false;
        agent.status = 'idle';
      }
      if (agentManager.selectedAgentId === message.agent_id) {
        setAgentStatus('connected', `${message.agent_id} completed`);
      }
      renderAgentRoster();
      renderAgentTranscript();
      break;
    }
    case 'agent_created': {
      agentManager.pendingCreate = false;
      agentManager.selectedAgentId = message.agent_id;
      setAgentStatus('connected', `Created ${message.agent_id} (${message.label})`);
      sendAgentMessage({ type: 'snapshot' });
      break;
    }
    case 'agent_deleted': {
      agentManager.agents.delete(message.agent_id);
      agentManager.attachedAgents.delete(message.agent_id);
      if (agentManager.selectedAgentId === message.agent_id) {
        agentManager.selectedAgentId = agentManager.agents.keys().next().value || null;
      }
      renderAgentRoster();
      renderAgentTranscript();
      setAgentStatus('connected', `Deleted ${message.agent_id}`);
      sendAgentMessage({ type: 'snapshot' });
      break;
    }
    case 'failed': {
      const agent = agentManager.agents.get(message.agent_id);
      if (agent) {
        agent.busy = false;
        agent.status = 'error';
      }
      setAgentStatus('error', message.message || 'Agent failed');
      renderAgentRoster();
      renderAgentTranscript();
      break;
    }
    case 'error': {
      agentManager.pendingCreate = false;
      setAgentStatus('error', message.message || 'Agent error');
      break;
    }
  }
}

function createAgent() {
  if (!agentManager?.socketOpen) {
    setAgentStatus('error', 'Shared websocket is not connected');
    return;
  }
  const label = window.prompt('New agent label (optional):', '');
  if (label === null) return;
  const rolesInput = window.prompt('Roles (comma-separated, blank defaults to executor):', 'executor');
  if (rolesInput === null) return;
  const roles = rolesInput
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean);
  agentManager.pendingCreate = true;
  if (!sendAgentMessage({ type: 'create', label: label.trim() || null, roles })) {
    agentManager.pendingCreate = false;
    setAgentStatus('error', 'Failed to send create request');
    return;
  }
  setAgentStatus('connecting', 'Creating web agent… waiting for server confirmation');
}

function deleteSelectedAgent() {
  if (!agentManager?.selectedAgentId) {
    setAgentStatus('error', 'No agent selected');
    return;
  }
  if (agentManager.selectedAgentId === 'master') {
    setAgentStatus('error', 'master cannot be deleted');
    return;
  }
  if (!window.confirm(`Delete ${agentManager.selectedAgentId}?`)) return;
  if (!sendAgentMessage({ type: 'delete', agent_id: agentManager.selectedAgentId })) {
    setAgentStatus('error', 'Failed to send delete request');
    return;
  }
  setAgentStatus('connecting', `Deleting ${agentManager.selectedAgentId}… waiting for server confirmation`);
}

function submitAgentPrompt(event) {
  event.preventDefault();
  if (!agentManager?.selectedAgentId || !agentManager.socketOpen) return;
  const text = agentManager.composerEl?.value?.trim();
  if (!text) return;
  const agent = agentManager.agents.get(agentManager.selectedAgentId);
  if (agent?.busy) {
    setAgentStatus('error', `${agent.label || agent.agent_id} is already busy`);
    return;
  }
  if (!sendAgentMessage({
    type: 'prompt_submit',
    agent_id: agentManager.selectedAgentId,
    prompt: text,
  })) {
    setAgentStatus('error', 'Failed to send prompt');
    return;
  }
  if (agentManager.composerEl) {
    agentManager.composerEl.value = '';
    agentManager.composerEl.focus();
  }
  setAgentStatus('connecting', `Submitting prompt to ${agentManager.selectedAgentId}…`);
}

function initAgentManager() {
  const rosterEl = document.getElementById('agent-roster');
  const transcriptEl = document.getElementById('agent-transcript');
  const formEl = document.getElementById('agent-composer-form');
  const composerEl = document.getElementById('agent-composer');
  const submitEl = document.getElementById('agent-submit');
  const newEl = document.getElementById('agent-new');
  const deleteEl = document.getElementById('agent-delete');
  const statusEl = document.getElementById('agent-status');
  const detailsEl = document.getElementById('agent-details');
  const titleEl = document.getElementById('agent-selected-title');
  const reconnectEl = document.getElementById('agent-reconnect');
  if (!rosterEl || !transcriptEl || !formEl || !composerEl || !submitEl || !statusEl) return;

  agentManager = {
    rosterEl,
    transcriptEl,
    formEl,
    composerEl,
    submitEl,
    statusEl,
    detailsEl,
    titleEl,
    reconnectEl,
    newEl,
    deleteEl,
    agents: new Map(),
    attachedAgents: new Set(),
    selectedAgentId: null,
    socketOpen: false,
    pendingCreate: false,
  };

  formEl.addEventListener('submit', submitAgentPrompt);
  reconnectEl?.addEventListener('click', () => connectSharedSocket());
  newEl?.addEventListener('click', () => createAgent());
  deleteEl?.addEventListener('click', () => deleteSelectedAgent());
}

function initAppShell() {
  setupTabs();
  setupSidebarToggle();
  selectTab('tab-main');
  initSharedSocketManager();
  initTerminalManager();
  initAgentManager();
  connectSharedSocket();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initAppShell, { once: true });
} else {
  initAppShell();
}

import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

const config = document.getElementById('app-config').dataset;
const vmId = config.vmId;
const fmCsrfToken = config.csrfToken;
const fmUploadDir = config.uploadDir;
const fmUploadAction = config.uploadAction;

// ── Terminal ──────────────────────────────────────────────────────────────────

const term = new Terminal({ cursorBlink: true, theme: { background: '#000000' } });
const fitAddon = new FitAddon();
term.loadAddon(fitAddon);

const container = document.getElementById('term-container');
term.open(container);
fitAddon.fit();
term.focus();

const ws = new WebSocket(
  (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host + '/ws/' + vmId
);
ws.binaryType = 'arraybuffer';

function sendResize() {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
  }
}

term.onResize(sendResize);
ws.onopen = () => { term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); };
ws.onmessage = e => term.write(new Uint8Array(e.data));
ws.onclose = () => term.write('\r\n\x1b[2mconnection closed\x1b[0m\r\n');
new ResizeObserver(() => fitAddon.fit()).observe(container);

document.getElementById('reset-btn')?.addEventListener('click', () => {
  document.getElementById('reset-dialog').showModal();
});

let fmCurrentPath = fmUploadDir;
let fmOpened = false;

document.getElementById('files-toggle-btn').addEventListener('click', toggleFiles);
document.getElementById('files-close-btn').addEventListener('click', closePanel);
document.addEventListener('keydown', e => { if (e.key === 'Escape') closePanel(); });

function closePanel() {
  const panel = document.getElementById('files-panel');
  panel.classList.remove('flex');
  panel.classList.add('hidden');
}

function toggleFiles() {
  const panel = document.getElementById('files-panel');
  const isOpen = panel.classList.toggle('flex');
  panel.classList.toggle('hidden', !isOpen);
  if (isOpen && !fmOpened) {
    fmOpened = true;
    loadDir(fmCurrentPath);
  }
}

function loadDir(path) {
  const list = document.getElementById('files-list');
  list.innerHTML = '<div class="flex justify-center py-6 opacity-40 text-xs">Loading\u2026</div>';
  fetch('/sessions/' + vmId + '/ls?path=' + encodeURIComponent(path))
    .then(res => res.ok ? res.json() : res.text().then(msg => { throw new Error(msg); }))
    .then(data => { fmCurrentPath = path; renderEntries(path, data.entries); })
    .catch(err => {
      list.innerHTML = '<div class="flex justify-center py-6 text-error text-xs">' + (err.message || 'Error.') + '</div>';
    });
}

const ICON_DIR  = '\u25b8';
const ICON_FILE = '\u00b7';
const ICON_UP   = '\u2039';

function renderEntries(path, entries) {
  renderBreadcrumb(path);
  const list = document.getElementById('files-list');
  list.innerHTML = '';
  if (path !== fmUploadDir) {
    list.appendChild(buildEntryRow(ICON_UP, '..', 'opacity-50 flex-1 truncate', () => loadDir(parentPath(path))));
  }
  for (const entry of entries) {
    const entryPath = path.replace(/\/$/, '') + '/' + entry.name;
    if (entry.is_dir) {
      const row = buildEntryRow(ICON_DIR, entry.name, 'text-info flex-1 truncate', () => loadDir(entryPath));
      const dl = document.createElement('span');
      dl.className = 'text-xs opacity-40 hover:opacity-100 px-1 cursor-pointer';
      dl.title = 'Download as zip';
      dl.textContent = '↓';
      dl.onclick = e => { e.stopPropagation(); window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank'); };
      row.appendChild(dl);
      list.appendChild(row);
    } else {
      const row = buildEntryRow(ICON_FILE, entry.name, 'flex-1 truncate', () => { window.location.href = '/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath); });
      const size = document.createElement('span');
      size.className = 'text-xs opacity-50 whitespace-nowrap';
      size.textContent = formatSize(entry.size);
      row.appendChild(size);
      list.appendChild(row);
    }
  }
  if (entries.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'flex justify-center py-6 opacity-40 text-xs';
    empty.textContent = 'Empty directory';
    list.appendChild(empty);
  }
}

function renderBreadcrumb(path) {
  const breadcrumb = document.getElementById('files-breadcrumb');
  breadcrumb.innerHTML = '';
  const normalized = path.replace(/\/$/, '') || '/';
  const root = fmUploadDir.replace(/\/$/, '') || '/';
  if (!normalized.startsWith(root)) {
    breadcrumb.textContent = normalized;
    return;
  }
  const isAtRoot = normalized === root;
  if (isAtRoot) {
    const rootSpan = document.createElement('span');
    rootSpan.textContent = 'Home';
    breadcrumb.appendChild(rootSpan);
  } else {
    const rootBtn = document.createElement('button');
    rootBtn.className = 'hover:underline cursor-pointer';
    rootBtn.textContent = 'Home';
    rootBtn.onclick = () => loadDir(root);
    breadcrumb.appendChild(rootBtn);
  }
  const suffix = normalized.slice(root.length);
  const subParts = suffix.split('/').filter(Boolean);
  subParts.forEach((part, i) => {
    const sep = document.createElement('span');
    sep.className = 'opacity-40';
    sep.textContent = ' / ';
    breadcrumb.appendChild(sep);
    const isCurrent = i === subParts.length - 1;
    if (isCurrent) {
      const span = document.createElement('span');
      span.textContent = part;
      breadcrumb.appendChild(span);
    } else {
      const segPath = root + '/' + subParts.slice(0, i + 1).join('/');
      const btn = document.createElement('button');
      btn.className = 'hover:underline cursor-pointer';
      btn.textContent = part;
      btn.onclick = () => loadDir(segPath);
      breadcrumb.appendChild(btn);
    }
  });
}

function buildEntryRow(icon, name, nameClass, onclick) {
  const row = document.createElement('div');
  row.className = 'flex items-center gap-2 px-3 py-1.5 cursor-pointer border-b border-base-300 text-xs hover:bg-base-300';
  row.onclick = onclick;
  const iconEl = document.createElement('span');
  iconEl.className = 'opacity-40 shrink-0';
  iconEl.textContent = icon;
  row.appendChild(iconEl);
  const nameEl = document.createElement('span');
  nameEl.className = nameClass;
  nameEl.textContent = name;
  row.appendChild(nameEl);
  return row;
}

function parentPath(path) {
  const stripped = path.replace(/\/$/, '');
  const idx = stripped.lastIndexOf('/');
  const parent = idx <= 0 ? '/' : stripped.substring(0, idx);
  return parent.length < fmUploadDir.length ? fmUploadDir : parent;
}

function formatSize(n) {
  if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
  return n + ' B';
}

const filesListEl = document.getElementById('files-list');
filesListEl.addEventListener('dragover', e => { e.preventDefault(); });
filesListEl.addEventListener('drop', e => {
  e.preventDefault();
  const file = e.dataTransfer.files[0];
  if (file) uploadFile(file);
});

function uploadFile(file) {
  const status = document.getElementById('files-upload-status');
  status.className = 'text-xs';
  status.textContent = 'Uploading\u2026';
  const formData = new FormData();
  formData.append('csrf_token', fmCsrfToken);
  formData.append('path', fmCurrentPath.replace(/\/$/, '') + '/' + file.name);
  formData.append('file', file);
  fetch(fmUploadAction, { method: 'POST', body: formData })
    .then(res => {
      status.className = 'text-xs ' + (res.ok ? 'text-success' : 'text-error');
      status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
      if (res.ok) loadDir(fmCurrentPath);
    })
    .catch(() => { status.className = 'text-xs text-error'; status.textContent = 'Network error.'; })
    .finally(() => setTimeout(() => { status.textContent = ''; status.className = 'text-xs'; }, 3000));
}

document.getElementById('fm-file-input').addEventListener('change', function() {
  if (!this.files[0]) return;
  uploadFile(this.files[0]);
  this.value = '';
});

// ── Chat panel ────────────────────────────────────────────────────────────────

const wsBase = (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host;

let chatSessionId = null;
let chatWs = null;
let chatStreaming = false;
let pendingQuery = null;
let pendingSessionTitle = null;

// Current assistant message container (text node inside it)
let currentAssistantMsgEl = null;
let currentAssistantTextEl = null;

// Map tool_use_id → { resultEl, detailsEl, resultHeader, resultIcon, resultLabel, resultBody } for filling in tool results
const pendingToolUses = new Map();

document.getElementById('chat-toggle-btn').addEventListener('click', toggleChat);
document.getElementById('chat-close-btn').addEventListener('click', closeChatPanel);
document.getElementById('chat-new-btn').addEventListener('click', startNewSession);
document.getElementById('chat-history-btn').addEventListener('click', () => {
  const panel = document.getElementById('chat-sessions-panel');
  const nowHidden = panel.classList.toggle('hidden');
  if (!nowHidden) loadChatHistory();
});
document.getElementById('chat-send-btn').addEventListener('click', () => {
  const input = document.getElementById('chat-input');
  const content = input.value.trim();
  if (!content || chatStreaming) return;
  input.value = '';
  autoResizeChatInput();
  sendQuery(content);
});
document.getElementById('chat-input').addEventListener('keydown', e => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    document.getElementById('chat-send-btn').click();
  }
});
document.getElementById('chat-input').addEventListener('input', autoResizeChatInput);

function autoResizeChatInput() {
  const el = document.getElementById('chat-input');
  el.style.height = 'auto';
  el.style.height = Math.min(el.scrollHeight, 160) + 'px';
}

function closeChatPanel() {
  const panel = document.getElementById('chat-panel');
  panel.classList.remove('flex');
  panel.classList.add('hidden');
}

function toggleChat() {
  const panel = document.getElementById('chat-panel');
  const isOpen = panel.classList.toggle('flex');
  panel.classList.toggle('hidden', !isOpen);
  if (isOpen && !chatWs) connectChatWs();
}

function connectChatWs() {
  chatWs = new WebSocket(wsBase + '/sessions/' + vmId + '/chat');
  chatWs.onopen = () => {
    if (pendingQuery) {
      const { content } = pendingQuery;
      pendingQuery = null;
      sendQuery(content);
    }
  };
  chatWs.onmessage = e => {
    let event;
    try { event = JSON.parse(e.data); } catch { return; }
    handleChatEvent(event);
  };
  chatWs.onclose = () => {
    chatWs = null;
    chatStreaming = false;
    pendingQuery = null;
    unlockChatInput();
  };
}

function handleChatEvent(event) {
  // Capture session_id from whichever event first carries it
  if (event.session_id && !chatSessionId) {
    chatSessionId = event.session_id;
  }
  if (event.type === 'system' && event.subtype === 'init') {
    showThinkingIndicator();
  } else if (event.type === 'assistant' || (event.content && Array.isArray(event.content) && !event.content.some(b => b.type === 'tool_result'))) {
    removeThinkingIndicator();
    const blocks = event.content ?? event.message?.content ?? [];
    let hasToolUse = false;
    for (const block of blocks) {
      if (block.text) {
        appendToAssistantMessage(block.text);
      } else if (block.type === 'tool_use') {
        hasToolUse = true;
        sealAssistantMessage();
        appendToolUseBlock(block.id, block.name, block.input);
      }
    }
    if (!hasToolUse) {
      sealAssistantMessage();
      chatStreaming = false;
      unlockChatInput();
    }
  } else if (event.type === 'user' || (event.content && Array.isArray(event.content) && event.content.some(b => b.type === 'tool_result'))) {
    const blocks = event.content ?? event.message?.content ?? [];
    for (const block of blocks) {
      if (block.type === 'tool_result') {
        fillToolResult(block.tool_use_id, block.content, block.is_error);
      }
    }
  } else if (event.type === 'result' || event.type === 'done') {
    if (event.session_id) chatSessionId = event.session_id;
    sealAssistantMessage();
    removeThinkingIndicator();
    chatStreaming = false;
    unlockChatInput();
    refreshChatHistory();
  } else if (event.type === 'error') {
    removeThinkingIndicator();
    sealAssistantMessage();
    appendErrorMessage(event.message ?? String(event));
    chatStreaming = false;
    unlockChatInput();
  }
  scrollChatToBottom();
}

function sendQuery(content) {
  if (chatSessionId === null) {
    pendingSessionTitle = content.slice(0, 60);
  }
  if (chatWs && chatWs.readyState === WebSocket.CONNECTING) {
    appendUserMessage(content);
    sealAssistantMessage();
    chatStreaming = true;
    lockChatInput();
    showThinkingIndicator();
    pendingQuery = { content };
    return;
  }
  appendUserMessage(content);
  sealAssistantMessage();
  chatStreaming = true;
  lockChatInput();
  showThinkingIndicator();
  chatWs.send(JSON.stringify({ type: 'query', content, session_id: chatSessionId }));
}

// ── Message builders ──────────────────────────────────────────────────────────

function appendUserMessage(content) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'flex justify-end px-3 py-1';
  const bubble = document.createElement('div');
  bubble.className = 'max-w-xs rounded-2xl rounded-br-sm px-3 py-2 text-sm text-white whitespace-pre-wrap break-words';
  bubble.style.background = '#2563eb';
  bubble.textContent = content;
  row.appendChild(bubble);
  messages.appendChild(row);
  scrollChatToBottom();
}

function ensureAssistantMessage() {
  if (currentAssistantMsgEl) return;
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'px-3 py-1';

  const header = document.createElement('div');
  header.className = 'flex items-center gap-2 mb-1';
  const avatar = document.createElement('div');
  avatar.className = 'w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold text-white shrink-0';
  avatar.style.background = '#f97316';
  avatar.textContent = 'C';
  const label = document.createElement('span');
  label.className = 'text-xs font-medium';
  label.style.color = '#9ca3af';
  label.textContent = 'Claude';
  header.appendChild(avatar);
  header.appendChild(label);

  const textEl = document.createElement('div');
  textEl.className = 'text-sm pl-8 whitespace-pre-wrap break-words';
  textEl.style.color = '#e5e7eb';

  row.appendChild(header);
  row.appendChild(textEl);
  messages.appendChild(row);

  currentAssistantMsgEl = row;
  currentAssistantTextEl = textEl;
}

function appendToAssistantMessage(text) {
  ensureAssistantMessage();
  currentAssistantTextEl.textContent += text;
  scrollChatToBottom();
}

function sealAssistantMessage() {
  currentAssistantMsgEl = null;
  currentAssistantTextEl = null;
}

function appendToolUseBlock(toolId, toolName, input) {
  const messages = document.getElementById('chat-messages');
  const wrapper = document.createElement('div');
  wrapper.className = 'px-3 py-1 pl-11';

  const header = document.createElement('div');
  header.className = 'flex items-center gap-2 py-1 text-sm';
  header.style.color = '#9ca3af';
  const headerIcon = document.createElement('span');
  headerIcon.textContent = '⚙';
  headerIcon.style.color = '#60a5fa';
  const headerLabel = document.createElement('span');
  headerLabel.textContent = 'Using ' + toolName;
  header.appendChild(headerIcon);
  header.appendChild(headerLabel);

  const detailsEl = document.createElement('details');
  detailsEl.className = 'rounded overflow-hidden';
  detailsEl.style.cssText = 'border:1px solid #374151;margin-bottom:4px';
  const summary = document.createElement('summary');
  summary.className = 'flex items-center gap-1 px-2 py-1 cursor-pointer text-xs select-none';
  summary.style.cssText = 'background:#1f2937;color:#9ca3af;list-style:none';
  const arrow = document.createElement('span');
  arrow.textContent = '\u25b8';
  arrow.style.cssText = 'transition:transform .15s;display:inline-block';
  detailsEl.addEventListener('toggle', () => {
    arrow.style.transform = detailsEl.open ? 'rotate(90deg)' : '';
  });
  const summaryLabel = document.createElement('span');
  summaryLabel.textContent = 'View input parameters';
  summary.appendChild(arrow);
  summary.appendChild(summaryLabel);
  const pre = document.createElement('pre');
  pre.className = 'text-xs p-3 overflow-x-auto';
  pre.style.cssText = 'background:#111827;color:#d1fae5';
  pre.textContent = typeof input === 'object' ? JSON.stringify(input, null, 2) : String(input);
  detailsEl.appendChild(summary);
  detailsEl.appendChild(pre);

  const resultEl = document.createElement('div');
  resultEl.style.display = 'none';
  const resultHeader = document.createElement('div');
  resultHeader.className = 'flex items-center gap-1 text-xs py-1';
  resultHeader.style.color = '#10b981';
  const resultIcon = document.createElement('span');
  resultIcon.textContent = '\u2713';
  const resultLabel = document.createElement('span');
  resultLabel.textContent = 'Tool Result';
  resultHeader.appendChild(resultIcon);
  resultHeader.appendChild(resultLabel);
  const resultBody = document.createElement('div');
  resultBody.className = 'text-xs font-mono whitespace-pre-wrap';
  resultBody.style.cssText = 'color:#9ca3af;padding-left:4px';
  resultEl.appendChild(resultHeader);
  resultEl.appendChild(resultBody);

  wrapper.appendChild(header);
  wrapper.appendChild(detailsEl);
  wrapper.appendChild(resultEl);
  messages.appendChild(wrapper);

  pendingToolUses.set(toolId, { resultEl, detailsEl, resultHeader, resultIcon, resultLabel, resultBody });
  scrollChatToBottom();
}

function fillToolResult(toolId, content, isError) {
  const entry = pendingToolUses.get(toolId);
  if (!entry) return;
  pendingToolUses.delete(toolId);
  const { resultEl, detailsEl, resultHeader, resultIcon, resultLabel, resultBody } = entry;
  const text = Array.isArray(content)
    ? content.filter(c => c.type === 'text').map(c => c.text).join('')
    : String(content ?? '');
  const truncated = text.length > 500 ? text.slice(0, 500) + '...' : text;
  if (isError) {
    resultHeader.style.color = '#f87171';
    resultIcon.textContent = '\u2717';
    resultLabel.textContent = 'Error';
    detailsEl.open = true;
  }
  resultBody.textContent = truncated;
  resultEl.style.display = '';
}

function appendErrorMessage(msg) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'px-3 py-1 text-sm rounded-lg mx-3';
  row.style.cssText = 'color:#fca5a5;background:#450a0a;border:1px solid #7f1d1d';
  row.textContent = 'Error: ' + msg;
  messages.appendChild(row);
}

// ── Thinking indicator ────────────────────────────────────────────────────────

function showThinkingIndicator() {
  if (document.getElementById('chat-thinking')) return;
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.id = 'chat-thinking';
  row.className = 'flex items-center gap-2 px-3 py-1';
  const avatar = document.createElement('div');
  avatar.className = 'w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold text-white shrink-0';
  avatar.style.background = '#f97316';
  avatar.textContent = 'C';
  const dots = document.createElement('div');
  dots.className = 'flex gap-1';
  for (let i = 0; i < 3; i++) {
    const d = document.createElement('div');
    d.className = 'w-1.5 h-1.5 rounded-full';
    d.style.cssText = 'background:#6b7280;animation:chatDot 1.2s ease-in-out infinite;animation-delay:' + (i * 0.2) + 's';
    dots.appendChild(d);
  }
  row.appendChild(avatar);
  row.appendChild(dots);
  messages.appendChild(row);
  scrollChatToBottom();
}

function removeThinkingIndicator() {
  document.getElementById('chat-thinking')?.remove();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function scrollChatToBottom() {
  const messages = document.getElementById('chat-messages');
  messages.scrollTop = messages.scrollHeight;
}

function lockChatInput() {
  document.getElementById('chat-send-btn').disabled = true;
  document.getElementById('chat-input').disabled = true;
}

function unlockChatInput() {
  document.getElementById('chat-send-btn').disabled = false;
  document.getElementById('chat-input').disabled = false;
}

// ── Chat session history ───────────────────────────────────────────────────────

async function loadChatHistory() {
  const panel = document.getElementById('chat-sessions-panel');
  panel.innerHTML = '<div class="px-3 py-2 text-xs opacity-50">Loading\u2026</div>';
  try {
    const res = await fetch('/sessions/' + vmId + '/chat-history');
    if (!res.ok) throw new Error('Failed to load');
    const chatSessions = await res.json();
    renderChatHistory(chatSessions);
  } catch {
    panel.innerHTML = '<div class="px-3 py-2 text-xs" style="color:#f87171">Failed to load history</div>';
  }
}

function renderChatHistory(chatSessions) {
  const panel = document.getElementById('chat-sessions-panel');
  panel.innerHTML = '';
  if (chatSessions.length === 0) {
    panel.innerHTML = '<div class="px-3 py-2 text-xs opacity-50">No previous sessions</div>';
    return;
  }
  for (const chatSession of chatSessions) {
    panel.appendChild(buildChatSessionItem(chatSession));
  }
}

function buildChatSessionItem(chatSession) {
  const item = document.createElement('div');
  item.className = 'flex items-center justify-between px-3 py-2 cursor-pointer text-sm';
  item.style.cssText = 'border-bottom:1px solid #374151';
  item.addEventListener('mouseenter', () => { item.style.background = '#374151'; });
  item.addEventListener('mouseleave', () => { item.style.background = ''; });
  item.onclick = () => resumeSession(chatSession.session_id, chatSession.title);
  const titleSpan = document.createElement('span');
  titleSpan.className = 'truncate flex-1';
  titleSpan.style.color = '#e5e7eb';
  titleSpan.textContent = chatSession.title;
  const timeSpan = document.createElement('span');
  timeSpan.className = 'text-xs opacity-50 shrink-0 ml-2';
  timeSpan.textContent = formatRelativeTime(chatSession.last_active_at);
  item.appendChild(titleSpan);
  item.appendChild(timeSpan);
  return item;
}

function formatRelativeTime(isoString) {
  const diff = Math.floor((Date.now() - new Date(isoString).getTime()) / 1000);
  if (diff < 60) return 'just now';
  if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
  if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
  return new Date(isoString).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

function startNewSession() {
  chatSessionId = null;
  document.getElementById('chat-messages').innerHTML = '';
  document.getElementById('chat-sessions-panel').classList.add('hidden');
}

function resumeSession(sessionId) {
  chatSessionId = sessionId;
  document.getElementById('chat-messages').innerHTML = '';
  document.getElementById('chat-sessions-panel').classList.add('hidden');
}

function refreshChatHistory() {
  const panel = document.getElementById('chat-sessions-panel');
  if (!panel.classList.contains('hidden')) loadChatHistory();
}

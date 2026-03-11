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
let currentAssistantEl = null;
let pendingToolApprovals = 0;

document.getElementById('chat-toggle-btn').addEventListener('click', toggleChat);
document.getElementById('chat-close-btn').addEventListener('click', closeChatPanel);
document.getElementById('chat-send-btn').addEventListener('click', () => {
  const input = document.getElementById('chat-input');
  const content = input.value.trim();
  if (!content || chatStreaming) return;
  input.value = '';
  sendQuery(content);
});
document.getElementById('chat-input').addEventListener('keydown', e => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    document.getElementById('chat-send-btn').click();
  }
});

function closeChatPanel() {
  const panel = document.getElementById('chat-panel');
  panel.classList.remove('flex');
  panel.classList.add('hidden');
}

function toggleChat() {
  const panel = document.getElementById('chat-panel');
  const isOpen = panel.classList.toggle('flex');
  panel.classList.toggle('hidden', !isOpen);
  if (isOpen && !chatWs) {
    connectChatWs();
  }
}

function connectChatWs() {
  chatWs = new WebSocket(wsBase + '/sessions/' + vmId + '/chat');
  chatWs.onmessage = e => {
    const data = JSON.parse(e.data);
    if (data.type === 'session-started') {
      chatSessionId = data.session_id;
    } else if (data.type === 'text') {
      appendToAssistantBubble(data.content);
    } else if (data.type === 'tool-use') {
      currentAssistantEl = null;
      appendToolUseRow(data.tool_name, data.input);
    } else if (data.type === 'tool-result') {
      appendToolResultRow(data.output, data.is_error);
      pendingToolApprovals = Math.max(0, pendingToolApprovals - 1);
      if (pendingToolApprovals === 0) unlockSend();
    } else if (data.type === 'complete') {
      chatSessionId = data.session_id;
      currentAssistantEl = null;
      chatStreaming = false;
      unlockSend();
    } else if (data.type === 'error') {
      currentAssistantEl = null;
      appendErrorRow(data.message);
      chatStreaming = false;
      unlockSend();
    }
  };
  chatWs.onclose = () => {
    chatWs = null;
    chatStreaming = false;
    unlockSend();
  };
}

function sendQuery(content) {
  appendUserBubble(content);
  currentAssistantEl = null;
  chatStreaming = true;
  lockSend();
  chatWs.send(JSON.stringify({ type: 'query', content, session_id: chatSessionId }));
}

function lockSend() {
  document.getElementById('chat-send-btn').disabled = true;
  document.getElementById('chat-input').disabled = true;
}

function unlockSend() {
  document.getElementById('chat-send-btn').disabled = false;
  document.getElementById('chat-input').disabled = false;
}

function appendUserBubble(content) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'chat chat-end';
  const bubble = document.createElement('div');
  bubble.className = 'chat-bubble chat-bubble-primary text-sm whitespace-pre-wrap';
  bubble.textContent = content;
  row.appendChild(bubble);
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
}

function appendAssistantBubble() {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'chat chat-start';
  const bubble = document.createElement('div');
  bubble.className = 'chat-bubble text-sm whitespace-pre-wrap';
  row.appendChild(bubble);
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
  return bubble;
}

function appendToAssistantBubble(text) {
  if (!currentAssistantEl) {
    currentAssistantEl = appendAssistantBubble();
  }
  currentAssistantEl.textContent += text;
  document.getElementById('chat-messages').scrollTop = document.getElementById('chat-messages').scrollHeight;
}

function appendToolUseRow(name, input) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'bg-base-300 rounded text-xs font-mono p-2 flex flex-col gap-1';
  const header = document.createElement('div');
  header.className = 'flex items-center gap-2';
  const label = document.createElement('span');
  label.className = 'opacity-70';
  label.textContent = '\u25b8 ' + name + ': ' + JSON.stringify(input);
  header.appendChild(label);
  const allowBtn = document.createElement('button');
  allowBtn.className = 'btn btn-xs btn-success ml-auto';
  allowBtn.textContent = 'Allow';
  const denyBtn = document.createElement('button');
  denyBtn.className = 'btn btn-xs btn-error';
  denyBtn.textContent = 'Deny';
  allowBtn.addEventListener('click', () => {
    allowBtn.remove();
    denyBtn.remove();
    chatWs.send(JSON.stringify({ type: 'tool-response', allow: true }));
  });
  denyBtn.addEventListener('click', () => {
    allowBtn.remove();
    denyBtn.remove();
    chatWs.send(JSON.stringify({ type: 'tool-response', allow: false }));
    pendingToolApprovals = Math.max(0, pendingToolApprovals - 1);
    if (pendingToolApprovals === 0) unlockSend();
  });
  header.appendChild(allowBtn);
  header.appendChild(denyBtn);
  row.appendChild(header);
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
  pendingToolApprovals++;
  lockSend();
}

function appendToolResultRow(output, isError) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'opacity-60 text-xs font-mono pl-4 whitespace-pre-wrap' + (isError ? ' text-error' : '');
  row.textContent = output;
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
}

function appendErrorRow(message) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'text-error text-xs p-2 bg-base-300 rounded';
  row.textContent = 'Error: ' + message;
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
}


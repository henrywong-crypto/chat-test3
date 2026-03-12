import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { marked } from 'marked';
import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import xml from 'highlight.js/lib/languages/xml';
import css from 'highlight.js/lib/languages/css';
import sql from 'highlight.js/lib/languages/sql';
import go from 'highlight.js/lib/languages/go';
import java from 'highlight.js/lib/languages/java';
import cpp from 'highlight.js/lib/languages/cpp';
import yaml from 'highlight.js/lib/languages/yaml';
import ruby from 'highlight.js/lib/languages/ruby';
import markdown from 'highlight.js/lib/languages/markdown';
import diff from 'highlight.js/lib/languages/diff';

hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('js', javascript);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('ts', typescript);
hljs.registerLanguage('python', python);
hljs.registerLanguage('py', python);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('rs', rust);
hljs.registerLanguage('bash', bash);
hljs.registerLanguage('sh', bash);
hljs.registerLanguage('shell', bash);
hljs.registerLanguage('json', json);
hljs.registerLanguage('xml', xml);
hljs.registerLanguage('html', xml);
hljs.registerLanguage('css', css);
hljs.registerLanguage('sql', sql);
hljs.registerLanguage('go', go);
hljs.registerLanguage('java', java);
hljs.registerLanguage('cpp', cpp);
hljs.registerLanguage('c', cpp);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('yml', yaml);
hljs.registerLanguage('ruby', ruby);
hljs.registerLanguage('rb', ruby);
hljs.registerLanguage('markdown', markdown);
hljs.registerLanguage('diff', diff);

marked.use({
  gfm: true,
  breaks: false,
  renderer: (() => {
    const r = new marked.Renderer();
    r.code = ({ text, lang }) => {
      const language = lang && hljs.getLanguage(lang) ? lang : null;
      const highlighted = language
        ? hljs.highlight(text, { language }).value
        : hljs.highlightAuto(text).value;
      return `<pre><code class="hljs language-${language ?? 'plaintext'}">${highlighted}</code></pre>`;
    };
    return r;
  })(),
});

const config = document.getElementById('app-config').dataset;
const vmId = config.vmId;
const fmCsrfToken = config.csrfToken;
const fmUploadDir = config.uploadDir;
const fmUploadAction = config.uploadAction;
const wsBase = (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host;

// ── Theme Management ──────────────────────────────────────────────────────────

function initTheme() {
  const savedTheme = localStorage.getItem('theme') || 'dark';
  document.documentElement.setAttribute('data-theme', savedTheme);
}

function toggleTheme() {
  const html = document.documentElement;
  const currentTheme = html.getAttribute('data-theme') || 'dark';
  const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
  html.setAttribute('data-theme', newTheme);
  localStorage.setItem('theme', newTheme);
}

// Initialize theme before page loads
initTheme();

document.getElementById('theme-toggle-btn')?.addEventListener('click', toggleTheme);

// ── Terminal ──────────────────────────────────────────────────────────────────

const term = new Terminal({ cursorBlink: true, theme: { background: '#000000' } });
const fitAddon = new FitAddon();
term.loadAddon(fitAddon);

let shellInitialized = false;

function initShell() {
  if (shellInitialized) return;
  shellInitialized = true;
  const container = document.getElementById('term-container');
  term.open(container);
  fitAddon.fit();
  term.focus();
  const termWs = new WebSocket(wsBase + '/ws/' + vmId);
  termWs.binaryType = 'arraybuffer';
  function sendResize() {
    if (termWs.readyState === WebSocket.OPEN) {
      termWs.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
    }
  }
  term.onResize(sendResize);
  termWs.onopen = () => {
    term.onData(d => termWs.send(new TextEncoder().encode(d)));
    sendResize();
    termWs.send(new TextEncoder().encode('claude --resume\r'));
  };
  termWs.onmessage = e => term.write(new Uint8Array(e.data));
  termWs.onclose = () => term.write('\r\n\x1b[2mconnection closed\x1b[0m\r\n');
  new ResizeObserver(() => fitAddon.fit()).observe(container);
}

// Default to Chat tab — shell connects lazily on first visit
document.addEventListener('DOMContentLoaded', () => {
  switchToChat();
});

document.getElementById('reset-btn')?.addEventListener('click', () => {
  document.getElementById('reset-dialog').showModal();
});

let fmCurrentPath = fmUploadDir;
let fmOpened = false;

// Files panel now integrated with terminal tab
document.getElementById('files-close-btn').addEventListener('click', closePanel);
document.getElementById('files-toggle-btn').addEventListener('click', toggleFiles);
document.addEventListener('keydown', e => {
  if (e.key === 'Escape') {
    closePanel();
    if (chatStreaming) stopGeneration();
  }
});

function closePanel() {
  const panel = document.getElementById('files-panel');
  const toggleBtn = document.getElementById('files-toggle-btn');
  panel.classList.remove('flex');
  panel.classList.add('hidden');
  toggleBtn.style.display = 'flex';
}

function toggleFiles() {
  const panel = document.getElementById('files-panel');
  const toggleBtn = document.getElementById('files-toggle-btn');
  const isOpen = panel.classList.toggle('flex');
  panel.classList.toggle('hidden', !isOpen);
  toggleBtn.style.display = isOpen ? 'none' : 'flex';
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
      const row = buildEntryRow(ICON_FILE, entry.name, 'flex-1 truncate', () => { window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank'); });
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

let chatSessionId = null;
let chatEs = null;
let chatEsPending = null;
let chatStreaming = false;
let streamHadText = false;
let pendingSessionTitle = null;

// Current assistant message container (text node inside it)
let currentAssistantMsgEl = null;
let currentAssistantTextEl = null;
let currentAssistantRawText = '';

// Thinking block state
let currentThinkingEl = null;
let currentThinkingTextEl = null;
let currentThinkingRawText = '';
let streamInThinkingBlock = false;

// Map tool_use_id → { resultEl, inner, resultHeader, resultIcon, resultLabel, resultBody, toolName }
const pendingToolUses = new Map();

// ── Document attachments ──────────────────────────────────────────────────────

let chatAttachments = []; // [{name, path}]

document.getElementById('chat-attach-btn').addEventListener('click', () => {
  document.getElementById('chat-attach-input').click();
});

document.getElementById('chat-attach-input').addEventListener('change', function() {
  for (const file of this.files) uploadAttachment(file);
  this.value = '';
});

async function uploadAttachment(file) {
  const MAX_BYTES = 50 * 1024 * 1024; // 50 MB
  if (file.size > MAX_BYTES) {
    appendErrorMessage(`File too large: ${file.name} (${(file.size / 1024 / 1024).toFixed(1)} MB, max 50 MB)`);
    return;
  }
  const placeholder = { name: file.name, path: null };
  chatAttachments.push(placeholder);
  renderAttachmentChips();
  try {
    const form = new FormData();
    form.append('csrf_token', fmCsrfToken);
    form.append('file', file, file.name);
    const res = await fetch('/sessions/' + vmId + '/chat-upload', { method: 'POST', body: form });
    if (!res.ok) throw new Error('upload failed: ' + res.status);
    const data = await res.json();
    placeholder.path = data.path;
  } catch (_err) {
    const idx = chatAttachments.indexOf(placeholder);
    if (idx !== -1) chatAttachments.splice(idx, 1);
  }
  renderAttachmentChips();
}

function renderAttachmentChips() {
  const container = document.getElementById('chat-attachments');
  if (chatAttachments.length === 0) {
    container.classList.remove('flex');
    container.classList.add('hidden');
    return;
  }
  container.classList.remove('hidden');
  container.classList.add('flex');
  container.innerHTML = '';
  chatAttachments.forEach((att, i) => {
    const chip = document.createElement('div');
    chip.className = 'attach-chip';
    const nameEl = document.createElement('span');
    nameEl.textContent = (att.path ? '📄 ' : '⏳ ') + att.name;
    const removeBtn = document.createElement('button');
    removeBtn.className = 'attach-chip-remove';
    removeBtn.textContent = '×';
    removeBtn.addEventListener('click', () => {
      chatAttachments.splice(i, 1);
      renderAttachmentChips();
    });
    chip.appendChild(nameEl);
    chip.appendChild(removeBtn);
    container.appendChild(chip);
  });
}

function buildQueryWithAttachments(userMessage) {
  const ready = chatAttachments.filter(att => att.path);
  if (ready.length === 0) return userMessage;
  const pathList = ready.map((att, i) => `${i + 1}. ${att.path}`).join('\n');
  return `${userMessage}\n\n[Files provided at the following paths:]\n${pathList}`;
}

document.getElementById('tab-shell-icon').addEventListener('click', switchToShell);
document.getElementById('tab-chat-icon').addEventListener('click', switchToChat);
document.getElementById('chat-new-btn').addEventListener('click', startNewSession);
document.getElementById('chat-history-refresh-btn')?.addEventListener('click', loadChatHistory);
document.getElementById('chat-stop-btn').addEventListener('click', stopGeneration);
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

function switchToChat() {
  const shellView = document.getElementById('shell-view');
  const chatView = document.getElementById('chat-view');
  shellView.classList.add('hidden');
  shellView.classList.remove('flex');
  chatView.classList.remove('hidden');
  chatView.classList.add('flex');
  document.getElementById('tab-chat-icon').classList.add('icon-active');
  document.getElementById('tab-shell-icon').classList.remove('icon-active');
  // Hide files panel when switching to chat
  const filesPanel = document.getElementById('files-panel');
  filesPanel.classList.remove('flex');
  filesPanel.classList.add('hidden');
  // Hide files toggle button in chat view
  const toggleBtn = document.getElementById('files-toggle-btn');
  toggleBtn.style.display = 'none';
  // Show session panel when switching to chat
  const sessionPanel = document.querySelector('.session-panel');
  sessionPanel.style.display = 'flex';
  // Restore chat title
  if (chatSessionId) {
    const activeItem = document.querySelector(`.session-item[data-id="${chatSessionId}"]`);
    if (activeItem) {
      const title = activeItem.querySelector('.session-item-title')?.textContent || 'Chat';
      updateChatTitle(title);
    } else {
      updateChatTitle('New Chat');
    }
  } else {
    updateChatTitle('New Chat');
  }
  loadChatHistory();
  if (!chatEs) connectChatSse();
}

function switchToShell() {
  const shellView = document.getElementById('shell-view');
  const chatView = document.getElementById('chat-view');
  chatView.classList.add('hidden');
  chatView.classList.remove('flex');
  shellView.classList.remove('hidden');
  shellView.classList.add('flex');
  document.getElementById('tab-shell-icon').classList.add('icon-active');
  document.getElementById('tab-chat-icon').classList.remove('icon-active');
  // Hide session panel when switching to terminal
  const sessionPanel = document.querySelector('.session-panel');
  sessionPanel.style.display = 'none';
  // Update title to Terminal
  updateChatTitle('Terminal');
  initShell();
  fitAddon.fit();
  // Auto-open files panel with terminal
  const filesPanel = document.getElementById('files-panel');
  const toggleBtn = document.getElementById('files-toggle-btn');
  filesPanel.classList.remove('hidden');
  filesPanel.classList.add('flex');
  toggleBtn.style.display = 'none';
  if (!fmOpened) {
    fmOpened = true;
    loadDir(fmCurrentPath);
  }
}

function connectChatSse() {
  console.log('[chat] connecting SSE', vmId);
  chatEs = new EventSource('/sessions/' + vmId + '/chat-stream');
  chatEs.onopen = () => {
    console.log('[chat] SSE open');
    if (chatEsPending) {
      const content = chatEsPending;
      chatEsPending = null;
      postQuery(content);
    }
  };
  chatEs.onerror = (e) => {
    console.log('[chat] SSE error  readyState=' + chatEs.readyState, e);
    if (chatEs.readyState === EventSource.CLOSED) {
      console.log('[chat] SSE closed');
      chatEs = null;
    }
    chatStreaming = false;
    streamHadText = false;
    sealAssistantMessage();
    unlockChatInput();
  };
  chatEs.addEventListener('init', () => {
    console.log('[chat] event: init');
    showThinkingIndicator();
  });
  chatEs.addEventListener('text_delta', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: text_delta  len=' + payload.text.length);
    removeThinkingIndicator();
    streamHadText = true;
    appendToAssistantMessage(payload.text);
  });
  chatEs.addEventListener('thinking_delta', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: thinking_delta  len=' + payload.thinking.length);
    appendToThinkingBlock(payload.thinking);
  });
  chatEs.addEventListener('tool_start', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: tool_start  name=' + payload.name);
    sealAssistantMessage();
    appendToolUseBlock(payload.id, payload.name, payload.input ?? {});
  });
  chatEs.addEventListener('tool_result', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: tool_result  tool_use_id=' + payload.tool_use_id + '  is_error=' + payload.is_error);
    fillToolResult(payload.tool_use_id, payload.content, payload.is_error);
  });
  chatEs.addEventListener('done', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: done  session_id=' + payload.session_id);
    if (payload.session_id) chatSessionId = payload.session_id;
    streamHadText = false;
    sealAssistantMessage();
    removeThinkingIndicator();
    chatStreaming = false;
    unlockChatInput();
    scrollChatToBottom();
    loadChatHistory();
  });
  chatEs.addEventListener('error_event', e => {
    const payload = JSON.parse(e.data);
    console.log('[chat] event: error_event  message=' + (payload.message ?? String(payload)));
    streamHadText = false;
    removeThinkingIndicator();
    sealAssistantMessage();
    appendErrorMessage(payload.message ?? String(payload));
    chatStreaming = false;
    unlockChatInput();
    scrollChatToBottom();
  });
}

function prepareForQuery(content) {
  appendUserMessage(content);
  sealAssistantMessage();
  streamHadText = false;
  chatStreaming = true;
  lockChatInput();
  showThinkingIndicator();
}

function sendQuery(content) {
  const fullContent = buildQueryWithAttachments(content);
  if (chatAttachments.length > 0) {
    chatAttachments = [];
    renderAttachmentChips();
  }
  if (chatSessionId === null) {
    pendingSessionTitle = content.slice(0, 60);
  }
  prepareForQuery(content);
  if (!chatEs || chatEs.readyState === EventSource.CONNECTING) {
    console.log('[chat] SSE not ready (readyState=' + (chatEs?.readyState ?? 'null') + '), queuing query');
    chatEsPending = fullContent;
    if (!chatEs) connectChatSse();
    return;
  }
  postQuery(fullContent);
}

function postQuery(content) {
  console.log('[chat] posting query  content_len=' + content.length + '  session_id=' + chatSessionId);
  fetch('/sessions/' + vmId + '/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content, session_id: chatSessionId, csrf_token: fmCsrfToken }),
  }).then(res => {
    console.log('[chat] query response  status=' + res.status);
    if (!res.ok) {
      res.text().then(msg => {
        sealAssistantMessage();
        appendErrorMessage(msg || `Server error ${res.status}`);
        chatStreaming = false;
        unlockChatInput();
      });
    }
  }).catch(err => {
    console.log('[chat] query fetch error:', err);
    sealAssistantMessage();
    appendErrorMessage('Failed to send message: ' + err.message);
    chatStreaming = false;
    unlockChatInput();
  });
}

// ── Message builders ──────────────────────────────────────────────────────────

function appendUserMessage(content) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'flex justify-end px-3 py-1';
  const bubble = document.createElement('div');
  bubble.className = 'user-bubble max-w-xs rounded-2xl rounded-br-sm px-3 py-2 text-sm text-white whitespace-pre-wrap break-words';
  bubble.textContent = content;
  row.appendChild(bubble);
  messages.appendChild(row);
  scrollChatToBottom();
}

function ensureAssistantMessage() {
  if (currentAssistantMsgEl) return;
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'chat-msg px-3 py-1';

  const header = document.createElement('div');
  header.className = 'flex items-center gap-2 mb-1';
  const avatar = document.createElement('div');
  avatar.className = 'claude-avatar w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold text-white shrink-0';
  avatar.textContent = 'C';
  const label = document.createElement('span');
  label.className = 'text-xs font-medium';
  label.textContent = 'Claude';
  header.appendChild(avatar);
  header.appendChild(label);

  const textEl = document.createElement('div');
  textEl.className = 'text-sm pl-8 whitespace-pre-wrap break-words';

  row.appendChild(header);
  row.appendChild(textEl);
  messages.appendChild(row);

  currentAssistantMsgEl = row;
  currentAssistantTextEl = textEl;
}

function appendToAssistantMessage(text) {
  if (!text) return;
  ensureAssistantMessage();
  currentAssistantRawText += text;
  currentAssistantTextEl.className = 'markdown-body text-sm pl-8';
  currentAssistantTextEl.innerHTML = marked.parse(currentAssistantRawText);
  injectCodeCopyButtons(currentAssistantTextEl);
  scrollChatToBottom();
}

function sealAssistantMessage() {
  if (currentAssistantMsgEl && currentAssistantRawText) {
    attachMessageCopyButton(currentAssistantMsgEl, currentAssistantRawText);
  }
  currentAssistantMsgEl = null;
  currentAssistantTextEl = null;
  currentAssistantRawText = '';
  sealThinkingBlock();
}

function sealThinkingBlock() {
  currentThinkingEl = null;
  currentThinkingTextEl = null;
  currentThinkingRawText = '';
}

function ensureThinkingBlock() {
  if (currentThinkingEl) return;
  const messages = document.getElementById('chat-messages');
  const details = document.createElement('details');
  details.className = 'thinking-block';

  const summary = document.createElement('summary');
  summary.className = 'thinking-summary';
  const arrow = document.createElement('span');
  arrow.className = 'thinking-arrow';
  arrow.textContent = '▸';
  summary.appendChild(arrow);
  summary.appendChild(document.createTextNode('Thinking…'));
  details.appendChild(summary);

  const textEl = document.createElement('div');
  textEl.className = 'thinking-text';
  details.appendChild(textEl);
  messages.appendChild(details);

  currentThinkingEl = details;
  currentThinkingTextEl = textEl;
  currentThinkingRawText = '';
}

function appendToThinkingBlock(text) {
  if (!text) return;
  ensureThinkingBlock();
  currentThinkingRawText += text;
  currentThinkingTextEl.textContent = currentThinkingRawText;
}

function attachMessageCopyButton(msgEl, rawText) {
  const header = msgEl.querySelector('.flex.items-center.gap-2.mb-1');
  if (!header || header.querySelector('.msg-copy-btn')) return;
  const btn = document.createElement('button');
  btn.className = 'msg-copy-btn ml-auto text-xs';
  btn.title = 'Copy message';
  btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
  btn.addEventListener('click', () => {
    navigator.clipboard.writeText(rawText).then(() => {
      btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>';
      btn.style.color = '#34d399';
      setTimeout(() => {
        btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
        btn.style.color = '';
      }, 2000);
    });
  });
  header.appendChild(btn);
}

// ── Tool category helpers ─────────────────────────────────────────────────────

const TOOL_CATEGORIES = {
  bash:     ['Bash'],
  edit:     ['Edit', 'Write', 'MultiEdit'],
  search:   ['Grep', 'Glob', 'Read', 'LS', 'ToolSearch', 'WebSearch', 'WebFetch'],
  todo:     ['TodoWrite', 'TodoRead'],
  task:     ['TaskCreate', 'TaskUpdate', 'TaskList', 'TaskGet'],
  agent:    ['Task'],
  plan:     ['ExitPlanMode', 'exit_plan_mode'],
  question: ['AskUserQuestion'],
};

const TOOL_CATEGORY_COLORS = {
  bash:     '#22c55e',
  edit:     '#f59e0b',
  search:   '#9ca3af',
  todo:     '#8b5cf6',
  task:     '#8b5cf6',
  agent:    '#a855f7',
  plan:     '#6366f1',
  question: '#3b82f6',
  default:  '#6b7280',
};

const TOOL_ICONS = {
  bash:     '⬡',
  edit:     '✎',
  search:   '⌕',
  todo:     '✓',
  task:     '◈',
  agent:    '◎',
  plan:     '◆',
  question: '?',
  default:  '⚙',
};

function getToolCategory(toolName) {
  for (const [cat, tools] of Object.entries(TOOL_CATEGORIES)) {
    if (tools.includes(toolName)) return cat;
  }
  return 'default';
}

// ── Tool input rendering ──────────────────────────────────────────────────────

function appendToolUseBlock(toolId, toolName, input) {
  const messages = document.getElementById('chat-messages');
  const category = getToolCategory(toolName);
  const borderColor = TOOL_CATEGORY_COLORS[category];
  const icon = TOOL_ICONS[category];

  const wrapper = document.createElement('div');
  wrapper.className = 'px-3 py-1 pl-8';

  const inner = document.createElement('div');
  inner.className = 'tool-inner';
  inner.style.borderLeftColor = borderColor;

  if (toolName === 'Bash') {
    inner.appendChild(buildBashInput(input));
  } else if (toolName === 'Read') {
    inner.appendChild(buildReadInput(input));
  } else if (toolName === 'Grep' || toolName === 'Glob') {
    inner.appendChild(buildSearchInput(toolName, input));
  } else if (toolName === 'Edit') {
    inner.appendChild(buildEditDiff(input));
  } else if (toolName === 'Write') {
    inner.appendChild(buildWritePreview(input));
  } else {
    inner.appendChild(buildGenericInput(toolName, icon, input));
  }

  const resultEl = document.createElement('div');
  resultEl.style.display = 'none';
  resultEl.style.marginTop = '4px';
  const resultHeader = document.createElement('div');
  resultHeader.className = 'flex items-center gap-1 text-xs py-0.5';
  resultHeader.style.color = '#10b981';
  const resultIcon = document.createElement('span');
  resultIcon.textContent = '\u2713';
  const resultLabel = document.createElement('span');
  resultLabel.textContent = 'Result';
  resultHeader.appendChild(resultIcon);
  resultHeader.appendChild(resultLabel);
  const resultBody = document.createElement('div');
  resultBody.className = 'result-body text-xs font-mono whitespace-pre-wrap';
  resultEl.appendChild(resultHeader);
  resultEl.appendChild(resultBody);

  wrapper.appendChild(inner);
  wrapper.appendChild(resultEl);
  messages.appendChild(wrapper);

  pendingToolUses.set(toolId, { resultEl, inner, resultHeader, resultIcon, resultLabel, resultBody, toolName });
  scrollChatToBottom();
}

function buildBashInput(input) {
  const cmd = input?.command ?? input?.cmd ?? (typeof input === 'string' ? input : JSON.stringify(input));
  const row = document.createElement('div');
  row.className = 'flex items-start gap-2 my-0.5';

  const iconEl = document.createElement('span');
  iconEl.className = 'bash-icon shrink-0';
  iconEl.textContent = '⬡';

  const pill = document.createElement('div');
  pill.className = 'bash-pill';

  const prompt = document.createElement('span');
  prompt.className = 'bash-prompt';
  prompt.textContent = '$';

  const code = document.createElement('code');
  code.className = 'bash-code';
  code.textContent = cmd;

  const copyBtn = buildInlineCopyBtn(cmd);

  pill.appendChild(prompt);
  pill.appendChild(code);
  pill.appendChild(copyBtn);
  row.appendChild(iconEl);
  row.appendChild(pill);
  return row;
}

function buildReadInput(input) {
  const filePath = input?.file_path ?? input?.path ?? String(input);
  const fileName = filePath.split('/').pop();
  const row = document.createElement('div');
  row.className = 'flex items-center gap-1.5 py-0.5 text-xs';
  const iconEl = document.createElement('span');
  iconEl.className = 'tool-search-icon';
  iconEl.textContent = '⌕';
  const nameEl = document.createElement('span');
  nameEl.className = 'tool-mono';
  nameEl.textContent = fileName;
  nameEl.title = filePath;
  const labelEl = document.createElement('span');
  labelEl.className = 'tool-dim';
  labelEl.textContent = 'Read';
  row.appendChild(iconEl);
  row.appendChild(nameEl);
  row.appendChild(labelEl);
  return row;
}

function buildSearchInput(toolName, input) {
  const pattern = input?.pattern ?? input?.glob ?? '';
  const path = input?.path ?? input?.directory ?? '';
  const row = document.createElement('div');
  row.className = 'flex items-center gap-1.5 py-0.5 text-xs';
  const iconEl = document.createElement('span');
  iconEl.className = 'tool-search-icon';
  iconEl.textContent = '⌕';
  const patternEl = document.createElement('span');
  patternEl.className = 'tool-pattern';
  patternEl.textContent = pattern;
  const labelEl = document.createElement('span');
  labelEl.className = 'tool-dim';
  labelEl.textContent = toolName;
  row.appendChild(iconEl);
  row.appendChild(patternEl);
  if (path) {
    const inEl = document.createElement('span');
    inEl.className = 'tool-dim';
    inEl.textContent = 'in';
    const pathEl = document.createElement('span');
    pathEl.className = 'tool-path-text';
    pathEl.textContent = path;
    pathEl.title = path;
    row.appendChild(inEl);
    row.appendChild(pathEl);
  }
  row.appendChild(labelEl);
  return row;
}

function buildEditDiff(input) {
  const filePath = input?.file_path ?? '';
  const oldStr = input?.old_string ?? '';
  const newStr = input?.new_string ?? '';
  const fileName = filePath.split('/').pop() || filePath;

  const container = document.createElement('div');
  container.className = 'tool-file-block';

  // Header
  const headerEl = document.createElement('div');
  headerEl.className = 'tool-file-header';
  const fileEl = document.createElement('span');
  fileEl.className = 'tool-file-name';
  fileEl.textContent = fileName;
  fileEl.title = filePath;
  const badge = document.createElement('span');
  badge.className = 'tool-badge';
  badge.textContent = 'Edit';
  headerEl.appendChild(fileEl);
  headerEl.appendChild(badge);
  container.appendChild(headerEl);

  // Diff lines
  const diffEl = document.createElement('div');
  diffEl.className = 'diff-lines';
  const oldLines = oldStr.split('\n');
  const newLines = newStr.split('\n');
  oldLines.forEach(line => diffEl.appendChild(buildDiffLine('-', line, false)));
  newLines.forEach(line => diffEl.appendChild(buildDiffLine('+', line, true)));
  container.appendChild(diffEl);
  return container;
}

function buildDiffLine(sign, content, isAdd) {
  const row = document.createElement('div');
  row.className = 'flex';
  const sigEl = document.createElement('span');
  sigEl.className = `diff-sign ${isAdd ? 'diff-sign-add' : 'diff-sign-del'}`;
  sigEl.textContent = sign;
  const textEl = document.createElement('span');
  textEl.className = `diff-text ${isAdd ? 'diff-text-add' : 'diff-text-del'}`;
  textEl.textContent = content;
  row.appendChild(sigEl);
  row.appendChild(textEl);
  return row;
}

function buildWritePreview(input) {
  const filePath = input?.file_path ?? '';
  const content = input?.content ?? '';
  const fileName = filePath.split('/').pop() || filePath;
  const lines = content.split('\n');
  const preview = lines.slice(0, 20).join('\n') + (lines.length > 20 ? '\n…' : '');

  const container = document.createElement('div');
  container.className = 'tool-file-block';

  const headerEl = document.createElement('div');
  headerEl.className = 'tool-file-header';
  const fileEl = document.createElement('span');
  fileEl.className = 'tool-file-name';
  fileEl.textContent = fileName;
  fileEl.title = filePath;
  const badge = document.createElement('span');
  badge.className = 'tool-badge tool-badge-new';
  badge.textContent = 'New file';
  headerEl.appendChild(fileEl);
  headerEl.appendChild(badge);
  container.appendChild(headerEl);

  const pre = document.createElement('pre');
  pre.className = 'write-preview';
  pre.textContent = preview;
  container.appendChild(pre);
  return container;
}

function buildGenericInput(toolName, icon, input) {
  // Build a short human-readable summary from the first 1-2 meaningful input fields
  const summary = summarizeInput(input);

  const wrapper = document.createElement('div');
  wrapper.className = 'generic-tool';

  const headerRow = document.createElement('div');
  headerRow.className = 'generic-tool-header';
  const iconEl = document.createElement('span');
  iconEl.textContent = icon;
  const nameEl = document.createElement('span');
  nameEl.className = 'generic-tool-name';
  nameEl.textContent = toolName;
  headerRow.appendChild(iconEl);
  headerRow.appendChild(nameEl);
  if (summary) {
    const sumEl = document.createElement('span');
    sumEl.className = 'generic-tool-summary';
    sumEl.textContent = summary;
    headerRow.appendChild(sumEl);
  }

  const details = document.createElement('details');
  const detailSummary = document.createElement('summary');
  detailSummary.className = 'generic-tool-detail-label';
  detailSummary.textContent = 'view params';
  const pre = document.createElement('pre');
  pre.className = 'generic-tool-params';
  pre.textContent = typeof input === 'object' ? JSON.stringify(input, null, 2) : String(input);
  details.appendChild(detailSummary);
  details.appendChild(pre);

  wrapper.appendChild(headerRow);
  wrapper.appendChild(details);
  return wrapper;
}

function summarizeInput(input) {
  if (!input || typeof input !== 'object') return String(input ?? '').slice(0, 80);
  // Pick the first string-valued key that looks meaningful
  const skip = new Set(['type', 'id', 'name']);
  for (const [k, v] of Object.entries(input)) {
    if (skip.has(k)) continue;
    if (typeof v === 'string' && v.length > 0) {
      const preview = v.slice(0, 80);
      return k + ': ' + (preview.length < v.length ? preview + '…' : preview);
    }
  }
  return '';
}

function buildInlineCopyBtn(text) {
  const btn = document.createElement('button');
  btn.className = 'inline-copy-btn';
  btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
  btn.addEventListener('click', () => {
    navigator.clipboard.writeText(text).then(() => {
      btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>';
      btn.style.color = '#34d399';
      setTimeout(() => {
        btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
        btn.style.color = '';
      }, 2000);
    });
  });
  return btn;
}

// ── Tool result rendering ─────────────────────────────────────────────────────

function fillToolResult(toolId, content, isError) {
  const entry = pendingToolUses.get(toolId);
  if (!entry) return;
  pendingToolUses.delete(toolId);
  const { resultEl, inner, resultHeader, resultIcon, resultLabel, resultBody, toolName } = entry;

  const category = getToolCategory(toolName);

  // Edit/Write: hide result on success — the diff already shows what changed
  if (!isError && (category === 'edit')) {
    return;
  }

  const text = Array.isArray(content)
    ? content.filter(c => c.type === 'text').map(c => c.text).join('')
    : String(content ?? '');

  if (isError) {
    // Red error box
    const errorBox = document.createElement('div');
    errorBox.className = 'tool-error-box';
    const errHeader = document.createElement('div');
    errHeader.className = 'tool-error-header';
    errHeader.innerHTML = '✗ Error';
    const errBody = document.createElement('div');
    errBody.className = 'tool-error-body';
    errBody.textContent = text;
    errorBox.appendChild(errHeader);
    errorBox.appendChild(errBody);
    inner.appendChild(errorBox);
    return;
  }

  // Bash: dark code block
  if (category === 'bash') {
    if (!text) return;
    const pre = document.createElement('pre');
    pre.className = 'bash-result';
    if (text.length > 400) {
      pre.textContent = text.slice(0, 400) + '…';
      const showMoreBtn = document.createElement('button');
      showMoreBtn.className = 'show-more-btn';
      showMoreBtn.textContent = 'show more';
      showMoreBtn.addEventListener('click', () => { pre.textContent = text; showMoreBtn.remove(); });
      inner.appendChild(pre);
      inner.appendChild(showMoreBtn);
    } else {
      pre.textContent = text;
      inner.appendChild(pre);
    }
    return;
  }

  // Default: plain text with show-more
  if (!text) return;
  resultHeader.style.color = '#10b981';
  resultIcon.textContent = '\u2713';
  resultLabel.textContent = 'Result';
  if (text.length > 300) {
    resultBody.textContent = text.slice(0, 300) + '…';
    const showMoreBtn = document.createElement('button');
    showMoreBtn.className = 'show-more-btn';
    showMoreBtn.textContent = 'show more';
    showMoreBtn.addEventListener('click', () => { resultBody.textContent = text; showMoreBtn.remove(); });
    resultEl.appendChild(showMoreBtn);
  } else {
    resultBody.textContent = text;
  }
  resultEl.style.display = '';
}

function injectCodeCopyButtons(container) {
  container.querySelectorAll('pre').forEach(pre => {
    if (pre.querySelector('.code-copy-btn')) return; // already injected
    pre.style.position = 'relative';
    const btn = document.createElement('button');
    btn.className = 'code-copy-btn';
    btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
    pre.appendChild(btn);
    btn.addEventListener('click', () => {
      const code = pre.querySelector('code')?.textContent ?? '';
      navigator.clipboard.writeText(code).then(() => {
        btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>';
        btn.style.color = '#34d399';
        setTimeout(() => {
          btn.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
          btn.style.color = '';
        }, 2000);
      });
    });
  });
}

function appendErrorMessage(msg) {
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.className = 'chat-error px-3 py-1 text-sm rounded-lg mx-3';
  row.textContent = 'Error: ' + msg;
  messages.appendChild(row);
}

// ── Thinking indicator ────────────────────────────────────────────────────────

let thinkingTimerInterval = null;
let thinkingStartTime = null;

function showThinkingIndicator() {
  if (document.getElementById('chat-thinking')) return;
  const messages = document.getElementById('chat-messages');
  const row = document.createElement('div');
  row.id = 'chat-thinking';
  row.className = 'flex items-center gap-2 px-3 py-1';
  const avatar = document.createElement('div');
  avatar.className = 'claude-avatar w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold text-white shrink-0';
  avatar.textContent = 'C';
  const dots = document.createElement('div');
  dots.className = 'flex gap-1';
  for (let i = 0; i < 3; i++) {
    const d = document.createElement('div');
    d.className = 'thinking-dot w-1.5 h-1.5 rounded-full';
    d.style.animationDelay = (i * 0.2) + 's';
    dots.appendChild(d);
  }
  const timerEl = document.createElement('span');
  timerEl.id = 'chat-thinking-timer';
  timerEl.className = 'thinking-timer text-xs';
  timerEl.textContent = '0s';
  row.appendChild(avatar);
  row.appendChild(dots);
  row.appendChild(timerEl);
  messages.appendChild(row);
  thinkingStartTime = Date.now();
  thinkingTimerInterval = setInterval(() => {
    const el = document.getElementById('chat-thinking-timer');
    if (el) el.textContent = Math.floor((Date.now() - thinkingStartTime) / 1000) + 's';
  }, 1000);
  scrollChatToBottom();
}

function removeThinkingIndicator() {
  clearInterval(thinkingTimerInterval);
  thinkingTimerInterval = null;
  thinkingStartTime = null;
  document.getElementById('chat-thinking')?.remove();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function extractContentBlocks(event) {
  const content = event.message?.content ?? event.content;
  if (Array.isArray(content)) return content;
  return [];
}

function scrollChatToBottom() {
  const scroll = document.getElementById('chat-scroll');
  scroll.scrollTop = scroll.scrollHeight;
}

function stopGeneration() {
  if (!chatStreaming) return;
  fetch('/sessions/' + vmId + '/chat/abort', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ csrf_token: fmCsrfToken }),
  });
}

function lockChatInput() {
  document.getElementById('chat-send-btn').disabled = true;
  document.getElementById('chat-send-btn').classList.add('hidden');
  document.getElementById('chat-stop-btn').classList.remove('hidden');
  document.getElementById('chat-input').disabled = true;
}

function unlockChatInput() {
  document.getElementById('chat-send-btn').disabled = false;
  document.getElementById('chat-send-btn').classList.remove('hidden');
  document.getElementById('chat-stop-btn').classList.add('hidden');
  document.getElementById('chat-input').disabled = false;
}

// ── Chat session history ───────────────────────────────────────────────────────

async function loadChatHistory() {
  const list = document.getElementById('chat-sessions-list');
  list.innerHTML = '<div class="px-3 py-2 text-xs opacity-50">Loading\u2026</div>';
  try {
    const res = await fetch('/sessions/' + vmId + '/chat-history');
    if (!res.ok) throw new Error('Failed to load');
    const chatSessions = await res.json();
    renderChatHistory(chatSessions);
  } catch {
    list.innerHTML = '<div class="px-3 py-2 text-xs" style="color:#f87171">Failed to load history</div>';
  }
}

function renderChatHistory(chatSessions) {
  const list = document.getElementById('chat-sessions-list');
  list.innerHTML = '';
  if (chatSessions.length === 0) {
    list.innerHTML = '<div class="px-3 py-2 text-xs opacity-50">No previous sessions</div>';
    return;
  }
  for (const chatSession of chatSessions) {
    list.appendChild(buildChatSessionItem(chatSession));
  }
}

function buildChatSessionItem(chatSession) {
  const isActive = chatSession.session_id === chatSessionId;
  const item = document.createElement('div');
  item.className = 'session-item' + (isActive ? ' active' : '');
  item.dataset.id = chatSession.session_id;
  item.onclick = () => resumeSession(chatSession.session_id, chatSession.title);

  const contentEl = document.createElement('div');
  contentEl.className = 'flex-1 min-w-0';

  const titleEl = document.createElement('div');
  titleEl.className = 'session-item-title';
  titleEl.textContent = chatSession.title;

  const statusEl = document.createElement('div');
  statusEl.className = 'session-item-status';
  statusEl.textContent = formatRelativeTime(chatSession.last_active_at);

  contentEl.appendChild(titleEl);
  contentEl.appendChild(statusEl);
  item.appendChild(contentEl);
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
  pendingToolUses.clear();
  streamInThinkingBlock = false;
  sealThinkingBlock();
  chatAttachments = [];
  renderAttachmentChips();
  document.getElementById('chat-messages').innerHTML = '';
  document.querySelectorAll('.session-item.active').forEach(el => el.classList.remove('active'));
  updateChatTitle(null);
}

function updateChatTitle(title) {
  const el = document.getElementById('chat-title');
  if (el) el.textContent = title ?? 'New Chat';
}

function resumeSession(sessionId, title) {
  chatSessionId = sessionId;
  pendingToolUses.clear();
  streamInThinkingBlock = false;
  sealThinkingBlock();
  chatAttachments = [];
  renderAttachmentChips();
  document.getElementById('chat-messages').innerHTML = '';
  document.querySelectorAll('.session-item.active').forEach(el => el.classList.remove('active'));
  const activeItem = document.querySelector(`.session-item[data-id="${sessionId}"]`);
  if (activeItem) activeItem.classList.add('active');
  updateChatTitle(title);
  loadAndRenderTranscript(sessionId);
}

async function loadAndRenderTranscript(sessionId) {
  try {
    const res = await fetch('/sessions/' + vmId + '/chat-transcript?session_id=' + encodeURIComponent(sessionId));
    if (!res.ok) return;
    const transcript = await res.json();
    renderTranscriptMessages(transcript.messages);
  } catch {}
}

function renderTranscriptMessages(messages) {
  for (const message of messages) {
    if (message.role === 'user') {
      const textContent = message.content
        .filter(b => b.type === 'text')
        .map(b => b.text)
        .join('');
      appendUserMessage(textContent);
    } else if (message.role === 'assistant') {
      for (const block of message.content) {
        if (block.type === 'text' && block.text) {
          appendToAssistantMessage(block.text);
        } else if (block.type === 'tool_use') {
          appendToolUseBlock(block.id, block.name, block.input);
        }
      }
      sealAssistantMessage();
    }
  }
}

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
document.addEventListener('keydown', e => {
  if (e.key === 'Escape') {
    closePanel();
    if (chatStreaming) stopGeneration();
  }
});

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

const wsBase = (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host;

let chatSessionId = null;
let chatWs = null;
let chatStreaming = false;
// True after we received text via stream_event deltas (avoid duplicating on full AssistantMessage)
let streamHadText = false;
let pendingQuery = null;
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
  } catch (err) {
    console.warn('[chat] attachment upload failed', file.name, err);
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
    chip.style.cssText = 'display:flex;align-items:center;gap:4px;padding:2px 8px;border-radius:12px;font-size:11px;color:#e5e7eb;background:#374151;border:1px solid #4b5563';
    const nameEl = document.createElement('span');
    nameEl.textContent = (att.path ? '📄 ' : '⏳ ') + att.name;
    const removeBtn = document.createElement('button');
    removeBtn.style.cssText = 'background:none;border:none;color:#9ca3af;cursor:pointer;font-size:13px;line-height:1;padding:0 0 0 2px';
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

document.getElementById('chat-toggle-btn').addEventListener('click', toggleChat);
document.getElementById('chat-close-btn').addEventListener('click', closeChatPanel);
document.getElementById('chat-new-btn').addEventListener('click', startNewSession);
document.getElementById('chat-history-btn').addEventListener('click', () => {
  const panel = document.getElementById('chat-sessions-panel');
  const nowHidden = panel.classList.toggle('hidden');
  if (!nowHidden) loadChatHistory();
});
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

function isChatPanelOpen() {
  return !document.getElementById('chat-panel').classList.contains('hidden');
}

function connectChatWs() {
  chatWs = new WebSocket(wsBase + '/sessions/' + vmId + '/chat');
  chatWs.onopen = () => {
    console.log('[chat] ws connected');
    if (pendingQuery) {
      const { content } = pendingQuery;
      pendingQuery = null;
      sendQuery(content);
    }
  };
  chatWs.onmessage = e => {
    console.log('[chat] raw ws message', e.data.slice(0, 200));
    let event;
    try { event = JSON.parse(e.data); } catch { console.warn('[chat] failed to parse ws message', e.data); return; }
    logChatEvent(event);
    handleChatEvent(event);
  };
  chatWs.onclose = () => {
    console.log('[chat] ws closed');
    chatWs = null;
    chatStreaming = false;
    streamHadText = false;
    pendingQuery = null;
    sealAssistantMessage();
    unlockChatInput();
    // Auto-reconnect if the chat panel is still open
    if (isChatPanelOpen()) {
      setTimeout(() => { if (!chatWs && isChatPanelOpen()) connectChatWs(); }, 2000);
    }
  };
}


/// Infer and inject missing `type` (and `delta.type`) into raw Anthropic streaming inner events.
/// The old agent.py on the VM may not carry these through Pydantic model_dump().
function normalizeStreamInnerEvent(ev) {
  if (!ev || typeof ev !== 'object') return ev;
  if (ev.type) {
    // delta.type might still be missing for content_block_delta
    if (ev.type === 'content_block_delta' && ev.delta && !ev.delta.type) {
      const delta = { ...ev.delta };
      if ('text' in delta) delta.type = 'text_delta';
      else if ('thinking' in delta) delta.type = 'thinking_delta';
      else if ('partial_json' in delta) delta.type = 'input_json_delta';
      return { ...ev, delta };
    }
    return ev;
  }
  // Infer outer type from structure
  let type = undefined;
  if (ev.delta !== undefined) type = 'content_block_delta';
  else if (ev.content_block !== undefined) type = 'content_block_start';
  else if (ev.message !== undefined) type = 'message_start';
  const normalized = type ? { ...ev, type } : ev;
  // Also fix delta.type
  if (normalized.type === 'content_block_delta' && normalized.delta && !normalized.delta.type) {
    const delta = { ...normalized.delta };
    if ('text' in delta) delta.type = 'text_delta';
    else if ('thinking' in delta) delta.type = 'thinking_delta';
    else if ('partial_json' in delta) delta.type = 'input_json_delta';
    return { ...normalized, delta };
  }
  return normalized;
}

function handleChatEvent(event) {
  // Capture session_id from whichever event first carries it
  if (event.session_id && !chatSessionId) {
    chatSessionId = event.session_id;
  }
  if (event.type === 'system' && event.subtype === 'init') {
    showThinkingIndicator();
  } else if (event.type === 'stream_event' && event.event) {
    const ev = normalizeStreamInnerEvent(event.event);
    if (ev.type === 'content_block_start') {
      const blockType = ev.content_block?.type;
      if (blockType === 'thinking') {
        streamInThinkingBlock = true;
        removeThinkingIndicator();
        ensureThinkingBlock();
      } else if (blockType === 'text') {
        streamInThinkingBlock = false;
      } else if (blockType === 'tool_use') {
        streamInThinkingBlock = false;
      }
    } else if (ev.type === 'content_block_stop') {
      if (streamInThinkingBlock) {
        sealThinkingBlock();
        streamInThinkingBlock = false;
      }
    } else if (ev.type === 'content_block_delta') {
      if (ev.delta?.type === 'thinking_delta' && ev.delta.thinking) {
        appendToThinkingBlock(ev.delta.thinking);
      } else if (ev.delta?.type === 'text_delta' && ev.delta.text) {
        removeThinkingIndicator();
        streamHadText = true;
        appendToAssistantMessage(ev.delta.text);
      }
    }
  } else if (event.type === 'assistant') {
    removeThinkingIndicator();
    const blocks = extractContentBlocks(event);
    for (const block of blocks) {
      if (block.type === 'text' && !streamHadText) {
        appendToAssistantMessage(block.text);
      } else if (block.type === 'tool_use') {
        sealAssistantMessage();
        appendToolUseBlock(block.id, block.name, block.input);
      }
    }
  } else if (event.type === 'user') {
    const blocks = extractContentBlocks(event);
    for (const block of blocks) {
      if (block.type === 'tool_result') {
        fillToolResult(block.tool_use_id, block.content, block.is_error);
      }
    }
  } else if (event.type === 'result' || event.type === 'done') {
    if (event.session_id) chatSessionId = event.session_id;
    // Fallback: if the SDK delivered text in result.result instead of streaming deltas, render it now.
    if (event.type === 'result' && typeof event.result === 'string' && event.result && !streamHadText) {
      appendToAssistantMessage(event.result);
    }
    streamHadText = false;
    sealAssistantMessage();
    removeThinkingIndicator();
    chatStreaming = false;
    unlockChatInput();
  } else if (event.type === 'error') {
    streamHadText = false;
    removeThinkingIndicator();
    sealAssistantMessage();
    appendErrorMessage(event.message ?? String(event));
    chatStreaming = false;
    unlockChatInput();
  }
  scrollChatToBottom();
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
  console.log('[chat] → query  session_id=', chatSessionId, ' content=', content.slice(0, 80));
  prepareForQuery(content); // show user's typed message only (not file content)
  if (chatWs && chatWs.readyState === WebSocket.CONNECTING) {
    pendingQuery = { content: fullContent };
    return;
  }
  chatWs.send(JSON.stringify({ type: 'query', content: fullContent, session_id: chatSessionId }));
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
  details.style.cssText = 'margin:4px 12px 0 12px;border-left:2px solid #374151;padding-left:10px';

  const summary = document.createElement('summary');
  summary.style.cssText = 'cursor:pointer;font-size:11px;color:#6b7280;user-select:none;list-style:none;display:flex;align-items:center;gap:4px;padding:2px 0';
  const arrow = document.createElement('span');
  arrow.style.cssText = 'font-size:9px;display:inline-block;transition:transform .15s';
  arrow.textContent = '▸';
  details.addEventListener('toggle', () => { arrow.style.transform = details.open ? 'rotate(90deg)' : ''; });
  summary.appendChild(arrow);
  summary.appendChild(document.createTextNode('🤔 Thinking…'));
  details.appendChild(summary);

  const textEl = document.createElement('div');
  textEl.style.cssText = 'font-size:11px;color:#6b7280;white-space:pre-wrap;word-break:break-words;padding:4px 0;line-height:1.5';
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
  btn.style.cssText = 'color:#6b7280;background:none;border:none;cursor:pointer;padding:0 2px;opacity:0;transition:opacity .15s';
  btn.title = 'Copy message';
  btn.textContent = '⎘';
  btn.addEventListener('click', () => {
    navigator.clipboard.writeText(rawText).then(() => {
      btn.textContent = '✓';
      btn.style.color = '#34d399';
      setTimeout(() => { btn.textContent = '⎘'; btn.style.color = '#6b7280'; }, 2000);
    });
  });
  header.appendChild(btn);
  msgEl.addEventListener('mouseenter', () => { btn.style.opacity = '1'; });
  msgEl.addEventListener('mouseleave', () => { btn.style.opacity = '0'; });
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
  inner.style.cssText = `border-left:2px solid ${borderColor};padding-left:10px;margin:2px 0`;

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
  resultBody.className = 'text-xs font-mono whitespace-pre-wrap';
  resultBody.style.cssText = 'color:#9ca3af;padding-left:4px';
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
  iconEl.style.cssText = 'color:#22c55e;font-size:11px;margin-top:3px;flex-shrink:0';
  iconEl.textContent = '⬡';

  const pill = document.createElement('div');
  pill.style.cssText = 'background:#0d1117;border-radius:5px;padding:4px 10px;flex:1;min-width:0;display:flex;align-items:center;gap:6px;overflow:hidden';

  const prompt = document.createElement('span');
  prompt.style.cssText = 'color:#166534;font-size:11px;font-family:monospace;flex-shrink:0;user-select:none';
  prompt.textContent = '$';

  const code = document.createElement('code');
  code.style.cssText = 'color:#4ade80;font-size:11px;font-family:monospace;white-space:pre-wrap;word-break:break-all';
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
  iconEl.style.color = '#9ca3af';
  iconEl.textContent = '⌕';
  const nameEl = document.createElement('span');
  nameEl.style.cssText = 'font-family:monospace;color:#60a5fa;font-size:11px';
  nameEl.textContent = fileName;
  nameEl.title = filePath;
  const labelEl = document.createElement('span');
  labelEl.style.color = '#4b5563';
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
  iconEl.style.color = '#9ca3af';
  iconEl.textContent = '⌕';
  const patternEl = document.createElement('span');
  patternEl.style.cssText = 'font-family:monospace;color:#e5e7eb;font-size:11px';
  patternEl.textContent = pattern;
  const labelEl = document.createElement('span');
  labelEl.style.color = '#4b5563';
  labelEl.textContent = toolName;
  row.appendChild(iconEl);
  row.appendChild(patternEl);
  if (path) {
    const inEl = document.createElement('span');
    inEl.style.color = '#4b5563';
    inEl.textContent = 'in';
    const pathEl = document.createElement('span');
    pathEl.style.cssText = 'font-family:monospace;color:#9ca3af;font-size:11px;max-width:180px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap';
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
  container.style.cssText = 'border:1px solid #374151;border-radius:5px;overflow:hidden;margin:2px 0;font-size:11px';

  // Header
  const headerEl = document.createElement('div');
  headerEl.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:3px 8px;background:#1f2937;border-bottom:1px solid #374151';
  const fileEl = document.createElement('span');
  fileEl.style.cssText = 'font-family:monospace;color:#60a5fa;font-size:11px';
  fileEl.textContent = fileName;
  fileEl.title = filePath;
  const badge = document.createElement('span');
  badge.style.cssText = 'font-size:10px;padding:1px 6px;border-radius:3px;background:#374151;color:#9ca3af';
  badge.textContent = 'Edit';
  headerEl.appendChild(fileEl);
  headerEl.appendChild(badge);
  container.appendChild(headerEl);

  // Diff lines
  const diffEl = document.createElement('div');
  diffEl.style.cssText = 'font-family:monospace;line-height:18px;max-height:300px;overflow-y:auto';
  const oldLines = oldStr.split('\n');
  const newLines = newStr.split('\n');
  oldLines.forEach(line => diffEl.appendChild(buildDiffLine('-', line, false)));
  newLines.forEach(line => diffEl.appendChild(buildDiffLine('+', line, true)));
  container.appendChild(diffEl);
  return container;
}

function buildDiffLine(sign, content, isAdd) {
  const row = document.createElement('div');
  row.style.cssText = 'display:flex';
  const sigEl = document.createElement('span');
  sigEl.style.cssText = `width:20px;text-align:center;flex-shrink:0;user-select:none;${
    isAdd
      ? 'background:#052e16;color:#4ade80'
      : 'background:#2d0505;color:#f87171'
  }`;
  sigEl.textContent = sign;
  const textEl = document.createElement('span');
  textEl.style.cssText = `flex:1;white-space:pre-wrap;word-break:break-all;padding:0 8px;${
    isAdd
      ? 'background:#031a0e;color:#bbf7d0'
      : 'background:#1a0505;color:#fecaca'
  }`;
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
  container.style.cssText = 'border:1px solid #374151;border-radius:5px;overflow:hidden;margin:2px 0;font-size:11px';

  const headerEl = document.createElement('div');
  headerEl.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:3px 8px;background:#1f2937;border-bottom:1px solid #374151';
  const fileEl = document.createElement('span');
  fileEl.style.cssText = 'font-family:monospace;color:#60a5fa;font-size:11px';
  fileEl.textContent = fileName;
  fileEl.title = filePath;
  const badge = document.createElement('span');
  badge.style.cssText = 'font-size:10px;padding:1px 6px;border-radius:3px;background:#052e16;color:#4ade80';
  badge.textContent = 'New file';
  headerEl.appendChild(fileEl);
  headerEl.appendChild(badge);
  container.appendChild(headerEl);

  const pre = document.createElement('pre');
  pre.style.cssText = 'margin:0;padding:6px 10px;background:#0d1117;color:#e5e7eb;font-size:11px;line-height:18px;max-height:200px;overflow-y:auto;white-space:pre-wrap;word-break:break-all';
  pre.textContent = preview;
  container.appendChild(pre);
  return container;
}

function buildGenericInput(toolName, icon, input) {
  // Build a short human-readable summary from the first 1-2 meaningful input fields
  const summary = summarizeInput(input);

  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'font-size:11px';

  const headerRow = document.createElement('div');
  headerRow.style.cssText = 'display:flex;align-items:center;gap:5px;color:#9ca3af;padding:2px 0';
  const iconEl = document.createElement('span');
  iconEl.textContent = icon;
  const nameEl = document.createElement('span');
  nameEl.style.cssText = 'color:#e5e7eb;font-weight:500';
  nameEl.textContent = toolName;
  headerRow.appendChild(iconEl);
  headerRow.appendChild(nameEl);
  if (summary) {
    const sumEl = document.createElement('span');
    sumEl.style.cssText = 'color:#6b7280;font-family:monospace;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:200px';
    sumEl.textContent = summary;
    headerRow.appendChild(sumEl);
  }

  const details = document.createElement('details');
  details.style.cssText = 'margin-top:2px';
  const detailSummary = document.createElement('summary');
  detailSummary.style.cssText = 'font-size:10px;color:#4b5563;cursor:pointer;list-style:none;user-select:none';
  detailSummary.textContent = 'view params';
  const pre = document.createElement('pre');
  pre.style.cssText = 'margin:4px 0 0;padding:6px 8px;background:#111827;color:#d1fae5;font-size:11px;border-radius:4px;overflow-x:auto;white-space:pre-wrap;word-break:break-all';
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
  btn.style.cssText = 'margin-left:auto;flex-shrink:0;font-size:10px;padding:1px 6px;border-radius:3px;border:1px solid #374151;background:#1f2937;color:#6b7280;cursor:pointer;opacity:0;transition:opacity .15s';
  btn.textContent = 'Copy';
  btn.addEventListener('click', () => {
    navigator.clipboard.writeText(text).then(() => {
      btn.textContent = '✓';
      btn.style.color = '#34d399';
      setTimeout(() => { btn.textContent = 'Copy'; btn.style.color = '#6b7280'; }, 2000);
    });
  });
  // Show on hover of parent pill
  btn.closest?.('div')?.addEventListener('mouseenter', () => { btn.style.opacity = '1'; });
  btn.closest?.('div')?.addEventListener('mouseleave', () => { btn.style.opacity = '0'; });
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
    errorBox.style.cssText = 'margin-top:4px;padding:6px 10px;border:1px solid #7f1d1d;border-radius:5px;background:#450a0a;font-size:11px';
    const errHeader = document.createElement('div');
    errHeader.style.cssText = 'display:flex;align-items:center;gap:4px;color:#f87171;margin-bottom:2px;font-weight:500';
    errHeader.innerHTML = '✗ Error';
    const errBody = document.createElement('div');
    errBody.style.cssText = 'color:#fecaca;white-space:pre-wrap;word-break:break-all;font-family:monospace';
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
    pre.style.cssText = 'margin-top:4px;padding:6px 10px;background:#0d1117;border:1px solid #374151;border-radius:5px;font-size:11px;font-family:monospace;color:#e5e7eb;white-space:pre-wrap;word-break:break-all;max-height:300px;overflow-y:auto';
    if (text.length > 400) {
      pre.textContent = text.slice(0, 400) + '…';
      const showMoreBtn = document.createElement('button');
      showMoreBtn.style.cssText = 'display:block;margin-top:4px;color:#60a5fa;background:none;border:none;cursor:pointer;font-size:11px;padding:0';
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
    showMoreBtn.style.cssText = 'display:block;margin-top:2px;color:#60a5fa;background:none;border:none;cursor:pointer;font-size:11px;padding:0';
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
    btn.textContent = 'Copy';
    btn.style.cssText = 'position:absolute;top:6px;right:8px;font-size:11px;padding:2px 8px;border-radius:4px;border:1px solid #374151;background:#1f2937;color:#9ca3af;cursor:pointer;opacity:0;transition:opacity .15s';
    pre.appendChild(btn);
    pre.addEventListener('mouseenter', () => { btn.style.opacity = '1'; });
    pre.addEventListener('mouseleave', () => { btn.style.opacity = '0'; });
    btn.addEventListener('click', () => {
      const code = pre.querySelector('code')?.textContent ?? '';
      navigator.clipboard.writeText(code).then(() => {
        btn.textContent = 'Copied!';
        btn.style.color = '#34d399';
        setTimeout(() => { btn.textContent = 'Copy'; btn.style.color = '#9ca3af'; }, 2000);
      });
    });
  });
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

let thinkingTimerInterval = null;
let thinkingStartTime = null;

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
  const timerEl = document.createElement('span');
  timerEl.id = 'chat-thinking-timer';
  timerEl.className = 'text-xs';
  timerEl.style.color = '#6b7280';
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

function logChatEvent(event) {
  const t = event.type;
  if (t === 'stream_event') {
    const inner = event.event;
    const innerType = inner?.type ?? '(no type)';
    if (innerType === 'content_block_delta') {
      const delta = inner?.delta;
      const preview = delta?.text ?? delta?.thinking ?? delta?.partial_json ?? '';
      console.log('[chat] ← stream_event  content_block_delta  delta.type=', delta?.type, ' text=', JSON.stringify(preview.slice(0, 40)));
    } else if (innerType === 'message_delta') {
      console.log('[chat] ← stream_event  message_delta', inner?.delta);
    } else {
      console.log('[chat] ← stream_event', innerType, inner);
    }
  } else if (t === 'assistant') {
    const blocks = extractContentBlocks(event).map(b => b.type);
    console.log('[chat] ← assistant  blocks=', blocks, ' session_id=', event.session_id);
  } else if (t === 'user') {
    const ids = extractContentBlocks(event)
      .filter(b => b.type === 'tool_result')
      .map(b => b.tool_use_id);
    console.log('[chat] ← user  tool_result_ids=', ids);
  } else if (t === 'result' || t === 'done') {
    console.log('[chat] ←', t, ' session_id=', event.session_id, ' result=', typeof event.result === 'string' ? JSON.stringify(event.result.slice(0, 80)) : event.result);
  } else if (t === 'error') {
    console.error('[chat] ← error', event.message, event);
  } else {
    console.log('[chat] ←', t, event.subtype ?? '', event);
  }
}

function scrollChatToBottom() {
  const messages = document.getElementById('chat-messages');
  messages.scrollTop = messages.scrollHeight;
}

function stopGeneration() {
  if (!chatStreaming || !chatWs) return;
  chatWs.send(JSON.stringify({ type: 'abort' }));
  // UI updates happen when the server closes the WS (onclose handler)
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
  pendingToolUses.clear();
  streamInThinkingBlock = false;
  sealThinkingBlock();
  chatAttachments = [];
  renderAttachmentChips();
  document.getElementById('chat-messages').innerHTML = '';
  document.getElementById('chat-sessions-panel').classList.add('hidden');
}

function resumeSession(sessionId) {
  chatSessionId = sessionId;
  pendingToolUses.clear();
  streamInThinkingBlock = false;
  sealThinkingBlock();
  chatAttachments = [];
  renderAttachmentChips();
  document.getElementById('chat-messages').innerHTML = '';
  document.getElementById('chat-sessions-panel').classList.add('hidden');
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

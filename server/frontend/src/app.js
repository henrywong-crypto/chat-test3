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
// True after we received text via stream_event deltas (avoid duplicating on full AssistantMessage)
let streamHadText = false;
let pendingQuery = null;
let pendingSessionTitle = null;

// Current assistant message container (text node inside it)
let currentAssistantMsgEl = null;
let currentAssistantTextEl = null;
let currentAssistantRawText = '';

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
  };
}

function handleChatEvent(event) {
  // Capture session_id from whichever event first carries it
  if (event.session_id && !chatSessionId) {
    chatSessionId = event.session_id;
  }
  if (event.type === 'system' && event.subtype === 'init') {
    showThinkingIndicator();
  } else if (event.type === 'stream_event' && event.event) {
    const ev = event.event;
    if (ev.type === 'content_block_delta' && ev.delta?.type === 'text_delta' && ev.delta.text) {
      removeThinkingIndicator();
      streamHadText = true;
      appendToAssistantMessage(ev.delta.text);
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
    streamHadText = false;
    sealAssistantMessage();
    removeThinkingIndicator();
    chatStreaming = false;
    unlockChatInput();
    refreshChatHistory();
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
  if (chatSessionId === null) {
    pendingSessionTitle = content.slice(0, 60);
  }
  console.log('[chat] → query  session_id=', chatSessionId, ' content=', content.slice(0, 80));
  prepareForQuery(content);
  if (chatWs && chatWs.readyState === WebSocket.CONNECTING) {
    pendingQuery = { content };
    return;
  }
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
  if (isError) {
    resultHeader.style.color = '#f87171';
    resultIcon.textContent = '\u2717';
    resultLabel.textContent = 'Error';
    detailsEl.open = true;
  }
  if (text.length > 300) {
    resultBody.textContent = text.slice(0, 300) + '…';
    const showMoreBtn = document.createElement('button');
    showMoreBtn.className = 'text-xs mt-1';
    showMoreBtn.style.cssText = 'color:#60a5fa;background:none;border:none;cursor:pointer;padding:0';
    showMoreBtn.textContent = 'show more';
    showMoreBtn.addEventListener('click', () => {
      resultBody.textContent = text;
      showMoreBtn.remove();
    });
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
    const inner = event.event?.type;
    // skip noisy per-character deltas
    if (inner === 'content_block_delta' || inner === 'message_delta') return;
    console.log('[chat] ←', t, inner);
  } else if (t === 'assistant') {
    const blocks = extractContentBlocks(event).map(b => b.type);
    console.log('[chat] ← assistant  blocks=', blocks, ' session_id=', event.session_id);
  } else if (t === 'user') {
    const ids = extractContentBlocks(event)
      .filter(b => b.type === 'tool_result')
      .map(b => b.tool_use_id);
    console.log('[chat] ← user  tool_result_ids=', ids);
  } else if (t === 'result' || t === 'done') {
    console.log('[chat] ←', t, ' session_id=', event.session_id);
  } else if (t === 'error') {
    console.error('[chat] ← error', event.message);
  } else {
    console.log('[chat] ←', t, event.subtype ?? '');
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
  document.getElementById('chat-messages').innerHTML = '';
  document.getElementById('chat-sessions-panel').classList.add('hidden');
}

function resumeSession(sessionId) {
  chatSessionId = sessionId;
  pendingToolUses.clear();
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

function refreshChatHistory() {
  const panel = document.getElementById('chat-sessions-panel');
  if (!panel.classList.contains('hidden')) loadChatHistory();
}

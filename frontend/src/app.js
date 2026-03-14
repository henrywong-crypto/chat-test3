import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

// ─── Config ───────────────────────────────────────────────────────────────────

function readConfig() {
  const el = document.getElementById("app-config");
  return {
    vmId:          el?.dataset.vmId ?? "",
    csrfToken:     el?.dataset.csrfToken ?? "",
    uploadDir:     el?.dataset.uploadDir ?? "/tmp",
    uploadAction:  el?.dataset.uploadAction ?? "",
    hasUserRootfs: el?.dataset.hasUserRootfs === "true",
  };
}

const cfg = readConfig();

// ─── State ────────────────────────────────────────────────────────────────────

const state = {
  activeTab:         "chat",
  sessions:          [],
  selectedSessionId: null,
  runningSessionId:  null,
  isStreaming:       false,
  messagesBySession: new Map(),
  pendingQuestion:   null,
  awaitingQuestion:  false,
  // SSE tracking
  _thinkingMsgId:    null,
  _assistantMsgId:   null,
  _toolIdToMsgId:    new Map(),
  // file manager
  files: { currentPath: cfg.uploadDir, entries: [], loading: false, error: null, uploadStatus: null },
  // question panel
  questionStep:          0,
  questionSelections:    new Map(),
  questionOtherTexts:    new Map(),
  questionOtherActive:   new Map(),
};

// ─── Message helpers ──────────────────────────────────────────────────────────

function genId() {
  return Math.random().toString(36).slice(2, 10);
}

function getMessages(sessionId) {
  return state.messagesBySession.get(sessionId) ?? [];
}

function setMessages(sessionId, msgs) {
  state.messagesBySession.set(sessionId, msgs);
}

function addMessage(sessionId, msg) {
  setMessages(sessionId, [...getMessages(sessionId), msg]);
}

function updateMessageById(sessionId, id, updater) {
  setMessages(sessionId, getMessages(sessionId).map(m => m.id === id ? updater(m) : m));
}

// ─── API ──────────────────────────────────────────────────────────────────────

async function post(path, body) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...body, csrf_token: cfg.csrfToken }),
  });
  if (!res.ok) throw new Error(await res.text() || `HTTP ${res.status}`);
}

async function sendQuery(content, sessionId) {
  await post(`/sessions/${cfg.vmId}/chat`, { content, session_id: sessionId });
}

async function sendStop() {
  await post(`/sessions/${cfg.vmId}/chat-stop`, {});
}

async function answerQuestion(requestId, answers) {
  await post(`/sessions/${cfg.vmId}/chat-question-answer`, { request_id: requestId, answers });
}

async function loadHistory() {
  const res = await fetch(`/sessions/${cfg.vmId}/chat-history`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function loadTranscript(sessionId, projectDir) {
  const params = new URLSearchParams({ session_id: sessionId, project_dir: projectDir });
  const res = await fetch(`/sessions/${cfg.vmId}/chat-transcript?${params}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const data = await res.json();
  return data.messages;
}

async function deleteSession(sessionId, projectDir) {
  const fd = new FormData();
  fd.append("csrf_token", cfg.csrfToken);
  fd.append("session_id", sessionId);
  fd.append("project_dir", projectDir);
  const res = await fetch(`/sessions/${cfg.vmId}/chat-transcript`, { method: "DELETE", body: fd });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
}

// ─── SSE ──────────────────────────────────────────────────────────────────────

function connectSse() {
  const es = new EventSource(`/sessions/${cfg.vmId}/chat-stream`);
  const on = (type, handler) =>
    es.addEventListener(type, e => handler(e.data ? JSON.parse(e.data) : null));

  on("init",              ()  => handleInit());
  on("text_delta",        (p) => handleTextDelta(p.text));
  on("thinking_delta",    (p) => handleThinkingDelta(p.thinking));
  on("tool_start",        (p) => handleToolStart(p));
  on("tool_result",       (p) => handleToolResult(p));
  on("ask_user_question", (p) => handleAskUserQuestion(p));
  on("done",              (p) => handleDone(p));
  on("error_event",       (p) => handleErrorEvent(p.message));
}

function handleInit() {
  const id = genId();
  state._thinkingMsgId  = id;
  state._assistantMsgId = null;
  addMessage(state.runningSessionId, { id, type: "assistant", content: "", isThinking: true, timestamp: Date.now() });
  renderMessages();
}

function handleTextDelta(text) {
  state._thinkingMsgId = null;
  if (!state._assistantMsgId) {
    const id = genId();
    state._assistantMsgId = id;
    addMessage(state.runningSessionId, { id, type: "assistant", content: text, timestamp: Date.now() });
  } else {
    updateMessageById(state.runningSessionId, state._assistantMsgId, m => ({ ...m, content: m.content + text }));
  }
  renderMessages();
}

function handleThinkingDelta(thinking) {
  if (state._thinkingMsgId) {
    updateMessageById(state.runningSessionId, state._thinkingMsgId, m => ({ ...m, content: m.content + thinking }));
    renderMessages();
  }
}

function handleToolStart({ id: toolId, name, input }) {
  state._thinkingMsgId  = null;
  state._assistantMsgId = null;
  if (name === "AskUserQuestion") return;
  const msgId = genId();
  state._toolIdToMsgId.set(toolId, msgId);
  addMessage(state.runningSessionId, {
    id: msgId, type: "tool", content: "", timestamp: Date.now(),
    isToolUse: true, toolId, toolName: name, toolInput: input,
  });
  renderMessages();
}

function handleToolResult({ tool_use_id, content, is_error }) {
  const msgId = state._toolIdToMsgId.get(tool_use_id);
  if (msgId) {
    updateMessageById(state.runningSessionId, msgId, m => ({
      ...m, toolResult: { content, isError: is_error },
    }));
    renderMessages();
  }
}

function handleAskUserQuestion({ request_id, questions }) {
  state._thinkingMsgId  = null;
  state._assistantMsgId = null;
  state.awaitingQuestion  = true;
  state.pendingQuestion   = { requestId: request_id, questions };
  state.questionStep      = 0;
  state.questionSelections  = new Map();
  state.questionOtherTexts  = new Map();
  state.questionOtherActive = new Map();
  renderComposerArea();
}

function handleDone({ session_id }) {
  const completedSession = state.runningSessionId;
  state.runningSessionId = null;
  state.isStreaming       = false;
  state.awaitingQuestion  = false;
  state.pendingQuestion   = null;
  state._thinkingMsgId    = null;
  state._assistantMsgId   = null;
  state._toolIdToMsgId.clear();

  if (session_id && completedSession === state.selectedSessionId) {
    setMessages(session_id, getMessages(completedSession));
    state.selectedSessionId = session_id;
  }

  loadHistory().then(sessions => { state.sessions = sessions; renderSidebar(); }).catch(console.error);
  renderComposerArea();
  renderMessages();
}

function handleErrorEvent(message) {
  state.runningSessionId = null;
  state.isStreaming       = false;
  state.awaitingQuestion  = false;
  state.pendingQuestion   = null;
  state._thinkingMsgId    = null;
  state._assistantMsgId   = null;
  state._toolIdToMsgId.clear();
  addMessage(state.selectedSessionId, { id: genId(), type: "error", content: message, timestamp: Date.now() });
  renderComposerArea();
  renderMessages();
}

// ─── Transcript ───────────────────────────────────────────────────────────────

function buildMessagesFromTranscript(transcript) {
  const msgs = [];
  let n = 0;
  const nextId = () => `t${n++}`;

  for (const entry of transcript) {
    if (entry.role === "user") {
      const content = typeof entry.content === "string"
        ? entry.content
        : entry.content.map(b => b.type === "text" ? b.text ?? "" : "").join("");
      if (content.trim()) msgs.push({ id: nextId(), type: "user", content, timestamp: Date.now() });
    } else if (entry.role === "assistant") {
      const blocks = typeof entry.content === "string"
        ? [{ type: "text", text: entry.content }]
        : entry.content;
      for (const block of blocks) {
        if (block.type === "thinking" && block.thinking) {
          msgs.push({ id: nextId(), type: "assistant", content: block.thinking, isThinking: true, timestamp: Date.now() });
        } else if (block.type === "text" && block.text) {
          msgs.push({ id: nextId(), type: "assistant", content: block.text, timestamp: Date.now() });
        } else if (block.type === "tool_use") {
          msgs.push({
            id: nextId(), type: "tool", content: "", timestamp: Date.now(),
            isToolUse: true, toolId: block.id, toolName: block.name, toolInput: block.input,
          });
        }
      }
    }
  }
  return msgs;
}

// ─── Markdown ─────────────────────────────────────────────────────────────────

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function renderMarkdown(text) {
  let html = "";
  const fence = /```[\s\S]*?```/g;
  let last = 0;
  let m;
  while ((m = fence.exec(text)) !== null) {
    if (m.index > last) html += renderInline(text.slice(last, m.index));
    const inner  = m[0].slice(3, -3);
    const nl     = inner.indexOf("\n");
    const code   = nl > 0 ? inner.slice(nl + 1) : inner;
    html += `<pre><code>${escapeHtml(code)}</code></pre>`;
    last = m.index + m[0].length;
  }
  if (last < text.length) html += renderInline(text.slice(last));
  return html;
}

function renderInline(text) {
  return text.split(/\n\n+/).map(para => {
    const t = para.trim();
    if (!t) return "";
    const header = t.match(/^(#{1,3}) (.+)/);
    if (header) return `<h${header[1].length}>${fmt(header[2])}</h${header[1].length}>`;
    if (/^[-*] /m.test(t)) {
      return "<ul>" + t.split("\n").filter(l => /^[-*] /.test(l.trim()))
        .map(l => `<li>${fmt(l.replace(/^[-*] /, "").trim())}</li>`).join("") + "</ul>";
    }
    if (/^\d+\. /m.test(t)) {
      return "<ol>" + t.split("\n").filter(l => /^\d+\. /.test(l.trim()))
        .map(l => `<li>${fmt(l.replace(/^\d+\. /, "").trim())}</li>`).join("") + "</ol>";
    }
    return `<p>${t.split("\n").map(fmt).join("<br>")}</p>`;
  }).join("");
}

function fmt(text) {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>")
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
}

// ─── Tool summary ─────────────────────────────────────────────────────────────

function buildToolSummary(name, input) {
  if (name === "Bash" || name === "shell") {
    const cmd = input.command ?? input.cmd;
    if (typeof cmd === "string") return cmd.slice(0, 80);
  }
  if (["Read", "Write", "Edit", "Glob"].includes(name)) {
    const path = input.file_path ?? input.path ?? input.pattern;
    if (typeof path === "string") return path.slice(0, 80);
  }
  if (name === "Grep") {
    const pat = input.pattern;
    if (typeof pat === "string") return `/${pat}/`;
  }
  return "";
}

// ─── Scroll tracking ──────────────────────────────────────────────────────────

let userScrolled = false;

// ─── Messages rendering ───────────────────────────────────────────────────────

function renderMessages() {
  const pane = document.getElementById("messages-pane");
  if (!pane) return;

  const messages = getMessages(state.selectedSessionId);
  if (messages.length === 0 && !state.isStreaming) {
    pane.innerHTML = `<div class="empty-chat"><div class="empty-chat-icon">💬</div><p>Start a new conversation</p></div>`;
    return;
  }

  pane.innerHTML = messages.map((msg, i) => renderMessageHtml(msg, messages[i - 1] ?? null)).join("");

  if (!userScrolled) pane.scrollTop = pane.scrollHeight;
}

function renderMessageHtml(msg, prev) {
  const time = new Date(msg.timestamp).toLocaleTimeString();

  if (msg.type === "user") {
    return `<div class="msg msg-user">
      <div class="bubble-user">
        <div class="bubble-text">${escapeHtml(msg.content)}</div>
        <div class="bubble-time">${time}</div>
      </div>
    </div>`;
  }

  if (msg.type === "error") {
    return `<div class="msg msg-error"><span class="error-label">Error: </span>${escapeHtml(msg.content)}</div>`;
  }

  if (msg.isThinking) {
    if (!msg.content) {
      return `<div class="msg"><div class="thinking-indicator">
        <div class="claude-avatar">C</div>
        <div class="thinking-dots">
          <span class="thinking-dot"></span><span class="thinking-dot"></span><span class="thinking-dot"></span>
        </div>
      </div></div>`;
    }
    return `<div class="msg"><details class="thinking-details">
      <summary>▸ Thinking…</summary>
      <div class="thinking-content">${escapeHtml(msg.content)}</div>
    </details></div>`;
  }

  if (msg.isToolUse) {
    const summary   = buildToolSummary(msg.toolName ?? "", msg.toolInput ?? {});
    const resultHtml = msg.toolResult ? renderToolResultHtml(msg.toolResult) : "";
    return `<div class="msg"><div class="tool-block">
      <details>
        <summary class="tool-summary">
          <span class="tool-icon">⚙</span>
          <span class="tool-name">${escapeHtml(msg.toolName ?? "")}</span>
          ${summary ? `<span class="tool-cmd">${escapeHtml(summary)}</span>` : ""}
        </summary>
        <div class="tool-input"><pre>${escapeHtml(JSON.stringify(msg.toolInput, null, 2))}</pre></div>
      </details>
      ${resultHtml}
    </div></div>`;
  }

  const isGrouped = prev && prev.type === msg.type && !msg.isToolUse && !prev.isToolUse;
  return `<div class="msg ${isGrouped ? "msg-grouped" : ""}">
    ${!isGrouped ? `<div class="msg-header">
      <div class="claude-avatar">C</div>
      <span class="msg-author">Claude</span>
      <span class="msg-time">${time}</span>
    </div>` : ""}
    <div class="msg-content ${isGrouped ? "" : "msg-content-indented"}">${renderMarkdown(msg.content)}</div>
  </div>`;
}

function renderToolResultHtml(result) {
  const isLong     = result.content.length > 200;
  const errorLabel = result.isError ? `<div class="tool-error-label">Error</div>` : "";
  const cls        = result.isError ? "tool-result-error" : "";
  const content    = escapeHtml(result.content);

  if (isLong) {
    return `<div class="tool-result ${cls}">
      ${errorLabel}
      <pre class="tool-result-pre tool-result-truncated">${content}</pre>
      <button class="tool-expand-btn" onclick="this.previousElementSibling.classList.toggle('tool-result-truncated');this.textContent=this.previousElementSibling.classList.contains('tool-result-truncated')?'Show more':'Show less'">Show more</button>
    </div>`;
  }
  return `<div class="tool-result ${cls}">${errorLabel}<pre class="tool-result-pre">${content}</pre></div>`;
}

// ─── Sidebar ──────────────────────────────────────────────────────────────────

function renderSidebar() {
  const sidebar = document.getElementById("sidebar");
  if (!sidebar) return;

  const sessionsHtml = state.sessions.length === 0
    ? `<div class="sidebar-empty">No chats yet</div>`
    : state.sessions.map(s => {
        const title    = s.title || `Session ${s.session_id.slice(0, 8)}`;
        const isActive = s.session_id === state.selectedSessionId;
        return `<div class="session-row ${isActive ? "session-active" : ""}" data-sid="${escapeHtml(s.session_id)}">
          <span class="session-title">${escapeHtml(title)}</span>
          <button class="session-delete" data-sid="${escapeHtml(s.session_id)}" data-dir="${escapeHtml(s.project_dir ?? "")}" title="Delete">✕</button>
        </div>`;
      }).join("");

  sidebar.innerHTML = `
    <div class="sidebar-header">
      <span class="sidebar-title">Chats</span>
      <button id="refresh-btn" class="icon-btn" title="Refresh">⟳</button>
    </div>
    <div class="sidebar-list">${sessionsHtml}</div>
    <div class="sidebar-footer">
      <button id="new-chat-btn" class="new-chat-btn">+ New Chat</button>
    </div>`;

  sidebar.querySelector("#refresh-btn").onclick = () =>
    loadHistory().then(s => { state.sessions = s; renderSidebar(); }).catch(console.error);

  sidebar.querySelector("#new-chat-btn").onclick = () => {
    state.selectedSessionId = null;
    renderSidebar();
    renderMessages();
    renderComposerArea();
  };

  sidebar.querySelectorAll(".session-row").forEach(row => {
    row.onclick = e => {
      if (e.target.classList.contains("session-delete")) return;
      const id      = row.dataset.sid;
      const session = state.sessions.find(s => s.session_id === id);
      if (!session) return;
      state.selectedSessionId = id;
      renderSidebar();
      renderMessages();
      renderComposerArea();
      if (session.project_dir && getMessages(id).length === 0) {
        loadTranscript(id, session.project_dir).then(t => {
          setMessages(id, buildMessagesFromTranscript(t));
          renderMessages();
        }).catch(console.error);
      }
    };
  });

  sidebar.querySelectorAll(".session-delete").forEach(btn => {
    btn.onclick = async e => {
      e.stopPropagation();
      const id  = btn.dataset.sid;
      const dir = btn.dataset.dir;
      if (!dir) return;
      try {
        await deleteSession(id, dir);
        state.sessions = state.sessions.filter(s => s.session_id !== id);
        if (state.selectedSessionId === id) state.selectedSessionId = null;
        renderSidebar();
        renderMessages();
      } catch (err) {
        console.error("Failed to delete session", err);
      }
    };
  });
}

// ─── Composer area ────────────────────────────────────────────────────────────

function renderComposerArea() {
  const area = document.getElementById("composer-area");
  if (!area) return;

  if (state.awaitingQuestion && state.pendingQuestion) {
    area.innerHTML = `<div class="question-wrapper"><div id="question-panel"></div></div>`;
    renderQuestionPanel();
    return;
  }

  const isLoading = state.isStreaming && state.runningSessionId === state.selectedSessionId;
  area.innerHTML = `<div class="composer">
    <div class="composer-inner">
      <textarea id="msg-input" placeholder="Message Claude…" rows="1" ${isLoading ? "disabled" : ""}></textarea>
      ${isLoading
        ? `<button id="stop-btn" class="btn-stop" title="Stop (Esc)">■</button>`
        : `<button id="send-btn" class="btn-send" title="Send">↑</button>`}
    </div>
    <p class="composer-hint">Enter to send · Shift+Enter for newline</p>
  </div>`;

  const textarea = area.querySelector("#msg-input");
  textarea.oninput = () => {
    textarea.style.height = "auto";
    textarea.style.height = Math.min(textarea.scrollHeight, 300) + "px";
  };
  textarea.onkeydown = e => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); doSend(textarea.value); }
  };

  const sendBtn = area.querySelector("#send-btn");
  if (sendBtn) sendBtn.onclick = () => doSend(textarea.value);

  const stopBtn = area.querySelector("#stop-btn");
  if (stopBtn) stopBtn.onclick = () => sendStop().catch(console.error);
}

function doSend(value) {
  const text = value.trim();
  if (!text || state.isStreaming) return;
  const sessionId = state.selectedSessionId;
  addMessage(sessionId, { id: genId(), type: "user", content: text, timestamp: Date.now() });
  state.runningSessionId = sessionId;
  state.isStreaming       = true;
  renderMessages();
  renderComposerArea();
  sendQuery(text, sessionId).catch(err => {
    addMessage(sessionId, { id: genId(), type: "error", content: String(err), timestamp: Date.now() });
    state.runningSessionId = null;
    state.isStreaming       = false;
    renderMessages();
    renderComposerArea();
  });
}

// ─── Question panel ───────────────────────────────────────────────────────────

function renderQuestionPanel() {
  const panel = document.getElementById("question-panel");
  if (!panel || !state.pendingQuestion) return;

  const { requestId, questions } = state.pendingQuestion;
  const step    = state.questionStep;
  const q       = questions[step];
  if (!q) return;

  const total    = questions.length;
  const isSingle = total === 1;
  const isLast   = step === total - 1;
  const isFirst  = step === 0;
  const multi    = q.multiSelect ?? false;
  const selected = state.questionSelections.get(step) ?? new Set();
  const isOtherOn = state.questionOtherActive.get(step) ?? false;

  const progressHtml = !isSingle ? `<div class="q-progress">${questions.map((_, i) =>
    `<button class="q-dot ${i === step ? "q-dot-active" : i < step ? "q-dot-done" : ""}" data-step="${i}"></button>`
  ).join("")}</div>` : "";

  const optionsHtml = q.options.map((opt, i) => {
    const isSel = selected.has(opt.label);
    return `<button class="q-option ${isSel ? "q-option-selected" : ""}" data-label="${escapeHtml(opt.label)}">
      <kbd class="q-kbd ${isSel ? "q-kbd-selected" : ""}">${i + 1}</kbd>
      <div class="q-option-text">
        <div class="q-option-label">${escapeHtml(opt.label)}</div>
        ${opt.description ? `<div class="q-option-desc">${escapeHtml(opt.description)}</div>` : ""}
      </div>
      ${isSel ? `<span class="q-check">✓</span>` : ""}
    </button>`;
  }).join("");

  const otherHtml = `
    <button class="q-option q-option-other ${isOtherOn ? "q-option-selected" : ""}" id="q-other-btn">
      <kbd class="q-kbd ${isOtherOn ? "q-kbd-selected" : ""}">0</kbd>
      <span class="q-option-label">Other...</span>
    </button>
    ${isOtherOn ? `<div class="q-other-input-wrap">
      <input id="q-other-input" type="text" class="q-other-input" placeholder="Type your answer…"
             value="${escapeHtml(state.questionOtherTexts.get(step) ?? "")}"/>
    </div>` : ""}`;

  panel.innerHTML = `<div class="q-panel" tabindex="-1">
    <div class="q-top-bar"></div>
    <div class="q-header">
      <div class="q-header-row">
        <span class="q-label">Claude needs your input</span>
        ${q.header ? `<span class="q-header-tag">${escapeHtml(q.header)}</span>` : ""}
        ${!isSingle ? `<span class="q-step">${step + 1}/${total}</span>` : ""}
      </div>
      ${progressHtml}
      <p class="q-question">${escapeHtml(q.question)}</p>
      ${multi ? `<span class="q-multi-hint">Select all that apply</span>` : ""}
    </div>
    <div class="q-options">${optionsHtml}${otherHtml}</div>
    <div class="q-footer">
      <button id="q-skip" class="q-skip">${isSingle ? "Skip" : "Skip all"} <span class="q-key">Esc</span></button>
      <div class="q-actions">
        ${!isSingle && !isFirst ? `<button id="q-back" class="q-btn-secondary">Back</button>` : ""}
        <button id="q-next" class="q-btn-primary">${isLast ? "Submit" : "Next"} <span class="q-key">Enter</span></button>
      </div>
    </div>
  </div>`;

  const qPanel = panel.querySelector(".q-panel");
  qPanel.focus();

  panel.querySelectorAll(".q-option[data-label]").forEach(btn => {
    btn.onclick = () => { toggleQOption(step, btn.dataset.label, multi); renderQuestionPanel(); };
  });

  const otherBtn = panel.querySelector("#q-other-btn");
  if (otherBtn) otherBtn.onclick = () => {
    toggleQOther(step, multi);
    renderQuestionPanel();
    if (state.questionOtherActive.get(step)) {
      setTimeout(() => panel.querySelector("#q-other-input")?.focus(), 0);
    }
  };

  const otherInput = panel.querySelector("#q-other-input");
  if (otherInput) {
    otherInput.focus();
    otherInput.oninput = e => state.questionOtherTexts.set(step, e.target.value);
    otherInput.onkeydown = e => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault();
        if (isLast) submitQuestion(); else { state.questionStep++; renderQuestionPanel(); }
      }
    };
  }

  panel.querySelectorAll(".q-dot").forEach(dot => {
    dot.onclick = () => { state.questionStep = parseInt(dot.dataset.step); renderQuestionPanel(); };
  });

  panel.querySelector("#q-skip").onclick = () => skipQuestion();
  panel.querySelector("#q-next").onclick = () => {
    if (isLast) submitQuestion(); else { state.questionStep++; renderQuestionPanel(); }
  };
  const backBtn = panel.querySelector("#q-back");
  if (backBtn) backBtn.onclick = () => { state.questionStep--; renderQuestionPanel(); };

  qPanel.onkeydown = e => {
    if (e.target instanceof HTMLInputElement) return;
    const num = parseInt(e.key);
    if (!isNaN(num) && num >= 1 && num <= q.options.length) {
      e.preventDefault();
      toggleQOption(step, q.options[num - 1].label, multi);
      renderQuestionPanel();
      return;
    }
    if (e.key === "0")      { e.preventDefault(); toggleQOther(step, multi); renderQuestionPanel(); return; }
    if (e.key === "Enter")  { e.preventDefault(); if (isLast) submitQuestion(); else { state.questionStep++; renderQuestionPanel(); } return; }
    if (e.key === "Escape") { e.preventDefault(); skipQuestion(); return; }
  };
}

function toggleQOption(stepIdx, label, multi) {
  const current = state.questionSelections.get(stepIdx) ?? new Set();
  if (multi) {
    if (current.has(label)) current.delete(label); else current.add(label);
    state.questionSelections.set(stepIdx, current);
  } else {
    state.questionSelections.set(stepIdx, new Set([label]));
    state.questionOtherActive.set(stepIdx, false);
  }
}

function toggleQOther(stepIdx, multi) {
  const wasActive = state.questionOtherActive.get(stepIdx) ?? false;
  state.questionOtherActive.set(stepIdx, !wasActive);
  if (!multi && !wasActive) state.questionSelections.set(stepIdx, new Set());
}

function buildAnswers() {
  const answers = {};
  state.pendingQuestion.questions.forEach((q, idx) => {
    const selected  = Array.from(state.questionSelections.get(idx) ?? []);
    const isOther   = state.questionOtherActive.get(idx) ?? false;
    const otherText = (state.questionOtherTexts.get(idx) ?? "").trim();
    if (isOther && otherText) selected.push(otherText);
    if (selected.length > 0) answers[q.question] = selected.join(", ");
  });
  return answers;
}

function submitQuestion() {
  const { requestId } = state.pendingQuestion;
  const answers = buildAnswers();
  state.awaitingQuestion = false;
  state.pendingQuestion  = null;
  renderComposerArea();
  answerQuestion(requestId, answers).catch(console.error);
}

function skipQuestion() {
  const { requestId } = state.pendingQuestion;
  state.awaitingQuestion = false;
  state.pendingQuestion  = null;
  renderComposerArea();
  answerQuestion(requestId, {}).catch(console.error);
}

// ─── Terminal ─────────────────────────────────────────────────────────────────

let termFitAddon   = null;
let termInitialized = false;

function initTerminal() {
  if (termInitialized) return;
  const container = document.getElementById("terminal-container");
  if (!container) return;
  termInitialized = true;

  const term     = new XTerm({ cursorBlink: true, theme: { background: "#000000" }, fontFamily: "monospace", fontSize: 14 });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(container);
  fitAddon.fit();
  term.focus();
  termFitAddon = fitAddon;

  const wsProto = location.protocol === "https:" ? "wss:" : "ws:";
  const ws      = new WebSocket(`${wsProto}//${location.host}/ws/${cfg.vmId}`);
  ws.binaryType = "arraybuffer";

  const sendResize = () => {
    if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "resize", rows: term.rows, cols: term.cols }));
  };
  term.onResize(sendResize);

  ws.onopen = () => {
    term.onData(d => ws.send(new TextEncoder().encode(d)));
    sendResize();
    ws.send(new TextEncoder().encode("claude --resume\r"));
  };
  ws.onmessage = e => term.write(new Uint8Array(e.data));
  ws.onclose   = () => term.write("\r\n\x1b[2mconnection closed\x1b[0m\r\n");

  new ResizeObserver(() => fitAddon.fit()).observe(container);
}

// ─── File manager ─────────────────────────────────────────────────────────────

function renderFilesView() {
  const main = document.getElementById("files-view");
  if (!main) return;

  const f           = state.files;
  const breadcrumb  = buildBreadcrumb(f.currentPath, cfg.uploadDir);

  const breadcrumbHtml = breadcrumb.map((part, i) =>
    `${i > 0 ? '<span class="bc-sep">›</span>' : ""}` +
    (part.path
      ? `<button class="bc-btn" data-path="${escapeHtml(part.path)}">${escapeHtml(part.label)}</button>`
      : `<span class="bc-current">${escapeHtml(part.label)}</span>`)
  ).join("");

  let listHtml = "";
  if (f.loading) {
    listHtml = `<div class="fm-status">Loading…</div>`;
  } else if (f.error) {
    listHtml = `<div class="fm-error">${escapeHtml(f.error)}</div>`;
  } else {
    if (f.currentPath !== cfg.uploadDir) {
      listHtml += `<div class="fm-row" data-type="up"><span class="fm-icon">‹</span><span class="fm-name fm-name-muted">..</span></div>`;
    }
    for (const entry of f.entries) {
      const entryPath = f.currentPath.replace(/\/$/, "") + "/" + entry.name;
      if (entry.is_dir) {
        listHtml += `<div class="fm-row" data-type="dir" data-path="${escapeHtml(entryPath)}">
          <span class="fm-icon">📁</span>
          <span class="fm-name fm-name-dir">${escapeHtml(entry.name)}</span>
          <a class="fm-action" href="/sessions/${cfg.vmId}/download?path=${encodeURIComponent(entryPath)}" target="_blank" onclick="event.stopPropagation()" title="Download as zip">↓</a>
        </div>`;
      } else {
        listHtml += `<div class="fm-row" data-type="file" data-path="${escapeHtml(entryPath)}">
          <span class="fm-icon">📄</span>
          <span class="fm-name">${escapeHtml(entry.name)}</span>
          <span class="fm-size">${formatSize(entry.size)}</span>
        </div>`;
      }
    }
    if (f.entries.length === 0) listHtml += `<div class="fm-status">Empty directory</div>`;
  }

  main.innerHTML = `
    <div class="fm-header">
      <span class="fm-title">Files</span>
      <label class="fm-upload-btn" title="Upload file">↑ Upload<input type="file" class="fm-file-input"/></label>
    </div>
    <div class="fm-breadcrumb">${breadcrumbHtml}</div>
    ${f.uploadStatus ? `<div class="fm-upload-status">${escapeHtml(f.uploadStatus)}</div>` : ""}
    <div class="fm-list" id="fm-list">${listHtml}</div>`;

  main.querySelector(".fm-file-input").onchange = e => {
    const file = e.target.files?.[0];
    if (file) { handleFileUpload(file); e.target.value = ""; }
  };

  main.querySelectorAll(".bc-btn").forEach(btn => { btn.onclick = () => loadDir(btn.dataset.path); });

  const list = main.querySelector("#fm-list");
  list.ondragover = e => e.preventDefault();
  list.ondrop = e => { e.preventDefault(); const file = e.dataTransfer.files[0]; if (file) handleFileUpload(file); };

  list.querySelectorAll(".fm-row").forEach(row => {
    const type = row.dataset.type;
    if (type === "up")   row.onclick = () => loadDir(parentPath(f.currentPath, cfg.uploadDir));
    if (type === "dir")  row.onclick = e => { if (e.target.tagName !== "A") loadDir(row.dataset.path); };
    if (type === "file") row.onclick = () => window.open(`/sessions/${cfg.vmId}/download?path=${encodeURIComponent(row.dataset.path)}`, "_blank");
  });
}

async function loadDir(path) {
  state.files.loading = true;
  state.files.error   = null;
  renderFilesView();
  try {
    const res  = await fetch(`/sessions/${cfg.vmId}/ls?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(await res.text());
    const data = await res.json();
    state.files.currentPath = path;
    state.files.entries     = data.entries;
  } catch (err) {
    state.files.error = String(err);
  } finally {
    state.files.loading = false;
    renderFilesView();
  }
}

async function handleFileUpload(file) {
  state.files.uploadStatus = "Uploading…";
  renderFilesView();
  const fd = new FormData();
  fd.append("csrf_token", cfg.csrfToken);
  fd.append("path", state.files.currentPath.replace(/\/$/, "") + "/" + file.name);
  fd.append("file", file);
  try {
    const res = await fetch(cfg.uploadAction, { method: "POST", body: fd });
    state.files.uploadStatus = res.ok ? "Uploaded." : "Upload failed.";
    if (res.ok) await loadDir(state.files.currentPath);
  } catch {
    state.files.uploadStatus = "Network error.";
  }
  renderFilesView();
  setTimeout(() => { state.files.uploadStatus = null; renderFilesView(); }, 3000);
}

function formatSize(n) {
  if (n >= 1048576) return (n / 1048576).toFixed(1) + " MB";
  if (n >= 1024)    return (n / 1024).toFixed(1) + " KB";
  return n + " B";
}

function parentPath(path, rootPath) {
  const stripped = path.replace(/\/$/, "");
  const idx      = stripped.lastIndexOf("/");
  const parent   = idx <= 0 ? "/" : stripped.substring(0, idx);
  return parent.length < rootPath.length ? rootPath : parent;
}

function buildBreadcrumb(path, rootPath) {
  const normalized = path.replace(/\/$/, "") || "/";
  const root       = rootPath.replace(/\/$/, "") || "/";
  const parts      = [{ label: "Home", path: normalized === root ? null : root }];
  if (normalized !== root) {
    const subParts = normalized.slice(root.length).split("/").filter(Boolean);
    subParts.forEach((part, i) => {
      const isCurrent = i === subParts.length - 1;
      const segPath   = root + "/" + subParts.slice(0, i + 1).join("/");
      parts.push({ label: part, path: isCurrent ? null : segPath });
    });
  }
  return parts;
}

// ─── Tab switching ────────────────────────────────────────────────────────────

function setActiveTab(tab) {
  state.activeTab = tab;

  document.querySelectorAll(".rail-btn[data-tab]").forEach(btn =>
    btn.classList.toggle("rail-btn-active", btn.dataset.tab === tab));

  const sidebar = document.getElementById("sidebar");
  if (sidebar) sidebar.style.display = tab === "chat" ? "flex" : "none";

  document.getElementById("chat-view").style.display     = tab === "chat"     ? "flex" : "none";
  document.getElementById("terminal-view").style.display = tab === "terminal" ? "flex" : "none";
  document.getElementById("files-view").style.display    = tab === "files"    ? "flex" : "none";

  if (tab === "terminal") { initTerminal(); setTimeout(() => termFitAddon?.fit(), 50); }
  if (tab === "files")    { loadDir(state.files.currentPath); }
}

// ─── Layout bootstrap ─────────────────────────────────────────────────────────

function buildLayout() {
  const root = document.getElementById("app");
  root.innerHTML = `
    <div id="icon-rail" class="icon-rail">
      <button class="rail-btn rail-btn-active" data-tab="chat" title="Chat">💬</button>
      <button class="rail-btn" data-tab="terminal" title="Terminal">&gt;_</button>
      <button class="rail-btn" data-tab="files" title="Files">📁</button>
      <div class="rail-spacer"></div>
      ${cfg.hasUserRootfs ? `<button id="reset-btn" class="rail-btn" title="Reset environment">↺</button>` : ""}
      <button id="logout-btn" class="rail-btn" title="Logout">⏻</button>
    </div>
    <div id="sidebar" class="sidebar"></div>
    <main class="main-content">
      <div id="chat-view" class="chat-view">
        <div id="messages-pane" class="messages-pane"></div>
        <div id="composer-area"></div>
      </div>
      <div id="terminal-view" class="terminal-view" style="display:none">
        <div id="terminal-container" class="terminal-container"></div>
      </div>
      <div id="files-view" class="files-view" style="display:none"></div>
    </main>
    ${cfg.hasUserRootfs ? `
    <div id="reset-modal" class="modal-overlay" style="display:none">
      <div class="modal">
        <h3 class="modal-title">Reset Environment?</h3>
        <p class="modal-body">This will permanently delete all your files and reset your workspace to a clean state.</p>
        <p class="modal-warning">Please backup your files before proceeding. This action cannot be undone.</p>
        <div class="modal-footer">
          <button id="reset-cancel" class="btn-secondary">Cancel</button>
          <form method="post" action="/rootfs/delete">
            <input type="hidden" name="csrf_token" value="${escapeHtml(cfg.csrfToken)}"/>
            <button type="submit" class="btn-destructive">Reset Environment</button>
          </form>
        </div>
      </div>
    </div>` : ""}`;

  document.querySelectorAll(".rail-btn[data-tab]").forEach(btn => {
    btn.onclick = () => setActiveTab(btn.dataset.tab);
  });
  document.getElementById("logout-btn").onclick = () => { location.href = "/logout"; };

  if (cfg.hasUserRootfs) {
    const resetBtn   = document.getElementById("reset-btn");
    const resetModal = document.getElementById("reset-modal");
    resetBtn.onclick   = () => { resetModal.style.display = "flex"; };
    resetModal.onclick = e => { if (e.target === resetModal) resetModal.style.display = "none"; };
    document.getElementById("reset-cancel").onclick = () => { resetModal.style.display = "none"; };
  }

  const pane = document.getElementById("messages-pane");
  pane.onwheel  = () => { userScrolled = (pane.scrollHeight - pane.scrollTop - pane.clientHeight) >= 40; };
  pane.onscroll = () => { if ((pane.scrollHeight - pane.scrollTop - pane.clientHeight) < 40) userScrolled = false; };
}

// ─── Init ─────────────────────────────────────────────────────────────────────

buildLayout();
renderSidebar();
renderMessages();
renderComposerArea();
connectSse();
loadHistory().then(sessions => { state.sessions = sessions; renderSidebar(); }).catch(console.error);

# Chat Architecture: End-to-End Flow

Traces a message from the user pressing Enter through to Claude's response in the browser, and covers concurrent sessions, page refreshes, and reconnects.

---

## System Overview

```
Browser  ──POST /chat──▶  Server (Axum)  ──SSH channel──▶  agent.py (/tmp/agent.sock)
         ◀──SSE stream──                 ◀──SSE lines───
```

Three tiers:

1. **Server** (Axum, Rust) — HTTP router. Authenticates requests, opens an SSH channel per query, streams agent events back as SSE.
2. **chat-agent** (Rust) — opens SSH channel to the VM, sends a JSON message to the agent, reads SSE lines back, forwards as an HTTP SSE response body.
3. **Agent daemon** (Python, `agent.py`) — listens on `/tmp/agent.sock`. Runs Claude SDK queries, emits SSE-formatted lines.

**Key design**: `POST /chat` returns an SSE stream directly. There is no persistent background connection. Each query opens its own SSH channel; the channel closes when `done` fires.

---

## Routes

| Method | Path | Description |
|--------|------|-------------|
| POST | `/chat` | Send message → returns SSE stream |
| GET | `/chat-stream/{taskId}` | Reconnect to in-progress task |
| POST | `/chat-question-answer` | Answer a pending question |
| POST | `/chat-stop` | Interrupt a running task |
| GET | `/chat-history` | List server-side sessions |
| GET | `/chat-transcript` | Load session transcript |
| DELETE | `/chat-transcript` | Delete a session |
| POST | `/chat-upload` | Upload a file to the VM |
| GET | `/ls` | List files in a directory |

---

## Conversation Model

Conversations are a **frontend concept** stored in `localStorage['conversations_{vmId}']`:

```typescript
interface Conversation {
  conversationId: string;   // stable frontend UUID (never changes)
  sessionId?: string;        // Claude Code session ID (set after first done)
  projectDir?: string;       // set after done via loadHistory()
  title?: string;            // set after done via loadHistory()
  createdAt: number;
}
```

- `conversationId` is the stable key for messages and UI routing.
- `sessionId` is passed to the backend as `session_id` to resume a Claude Code session.
- No message migration on `done` — the conversation UUID is stable throughout.

On mount, `syncConversationsFromHistory()` fetches `/chat-history` and creates local `Conversation` entries for any server sessions not yet in localStorage. The Refresh button triggers it manually.

---

## 1. Sending a Message

### Frontend: `handleSend`

1. If `viewConversationId` is null (blank state): calls `createConversation()` → new UUID → `setViewConversationId(newConv.id)` → notifies parent via `onConversationCreated` so `selectedConversation` stays in sync.
2. Adds `{ type: "user" }` message under `conversationId`.
3. Sets `runningConversationId = conversationId`, `isStreaming = true`.
4. Calls `sendQuery(text, conversationId, conversation.sessionId)`.

The conversation appears in the sidebar immediately (no title yet → shows "New chat…" in italic).

### POST /chat

Request body:
```json
{ "conversation_id": "uuid", "content": "...", "session_id": null, "work_dir": null, "csrf_token": "..." }
```

Response: `Content-Type: text/event-stream`. The server opens an SSH channel to the VM, sends the query, and streams events back. The first event is always `task_created`:

```
event: task_created
data: {"task_id":"<uuid>","conversation_id":"<uuid>"}

event: session_start
data: {"task_id":"<uuid>"}

...text_delta, tool_start, etc...

event: done
data: {"session_id":"claude-session-id","task_id":"<uuid>","conversation_id":"<uuid>"}
```

The stream closes after `done`.

### Agent query (over SSH)

The server sends a JSON line over the SSH channel:
```json
{"type":"query","task_id":"...","conversation_id":"...","content":"Hello","session_id":"abc"}
```

`agent.py` parses this, spawns `asyncio.create_task(run_query(...))`, emits SSE lines back over the channel.

---

## 2. Streaming the Response

```
agent.py          Browser
   │── session_start ──▶│
   │── init ────────────▶│  (thinking dots appear)
   │── thinking_delta ──▶│
   │── text_delta ──────▶│  (thinking sealed/removed if empty)
   │── tool_start ──────▶│
   │── tool_result ─────▶│
   │── done ────────────▶│  (stream closes)
```

### `SseContext` event handling

`SseContext.sendQuery` uses `fetch` (not `EventSource`) and reads the SSE body via `ReadableStream`. Each parsed event is pushed into `eventQueueRef` and `eventSeq` is incremented. `useSseHandlers` drains the queue on each `eventSeq` tick.

| SSE event | Action |
|---|---|
| `task_created` | Stores `task_id`; calls `setTaskId(conversationId, taskId)` |
| `session_start` | Writes `chat_running_task_{vmId}` to localStorage |
| `init` | Adds `{ type: "assistant", isThinking: true }` bubble |
| `text_delta` | Seals/removes empty thinking bubble; appends to assistant message |
| `thinking_delta` | Appends to thinking message |
| `tool_start` | Adds `{ type: "tool", isToolUse: true }` message |
| `tool_result` | Updates matching tool message with result |
| `ask_user_question` | `setSessionPendingQuestion(conversationId, …)`; stores to `localStorage['question_{requestId}']` |
| `done` | Clears localStorage. Sets `sessionId` on conversation. Calls `loadHistory()` to get `projectDir` + `title`. |
| `error_event` | Clears localStorage. Adds `{ type: "error" }` message. |

---

## 3. State Management: Multiple Concurrent Conversations

```typescript
const isCurrentRunning = isStreaming && runningConversationId === viewConversationId;
const isOtherRunning   = isStreaming && runningConversationId !== viewConversationId;
```

- `isCurrentRunning` → status bar shown, composer in loading state.
- `isOtherRunning` → different conversation is streaming; composer shown but disabled.

### Clicking "New Chat" mid-stream

1. `setSelectedConversation(null)` → `viewConversationId = null`.
2. The original stream continues under `runningConversationId = oldConvId`.
3. Status bar hides (viewing blank state). Sidebar placeholder row stays with pulsing indicator.
4. When `done` fires, `loadHistory()` updates the placeholder with real title.

---

## 4. Page Refresh and Reconnect

### localStorage persistence

On `session_start`:
```
chat_running_task_{vmId} = {"task_id":"...","running_session_id":"<conversationId>"}
```
On each message event:
```
chat_messages_task_{taskId} = JSON.stringify(messages)
```
Cleared on `done` or `error_event`.

### Reconnect flow

On mount, `SseProvider` checks `localStorage['chat_running_task_{vmId}']`. If found:

1. Pushes `{ type: "reconnecting", task_id, conversation_id }` event.
2. Opens `EventSource GET /chat-stream/{taskId}?conversation_id={conversationId}`.
3. Server sends `Hello { task_id, conversation_id }` to the agent → agent rebinds the writer and re-emits any pending question.

`reconnecting` handler in `useSseHandlers`:
- Sets `runningConversationId`, `isStreaming = true`, `viewConversationId = conversationId`.
- Restores messages from `chat_messages_task_{taskId}` immediately.
- Loads server transcript if `conversation.sessionId` + `conversation.projectDir` exist, prepending historical messages.

---

## 5. Ask-User-Question Flow

```
agent.py creates a pending future, emits ask_user_question
Browser: AskUserQuestionPanel shown, composer hidden
         stores to localStorage['question_{requestId}']
User answers → panel clears immediately → POST /chat-question-answer
Server → agent: {"type":"answer_question","answers":{...}}
agent.py: pending_question.set_result(answers) → run_query resumes
```

On conversation switch: `getQuestionsForConversation(conversationId)` scans localStorage for matching `question_*` entries and restores the panel.

---

## 6. Stopping a Stream

User clicks Stop → `sendStop` POSTs `/chat-stop` → server → agent: `{"type":"interrupt","task_id":"..."}` → `session.task.cancel()` → `CancelledError` in `run_query`'s `finally` → emits `done` → frontend clears running state.

---

## 7. Transcript Loading

When a conversation with `sessionId` + `projectDir` is selected, `loadTranscriptForConversation` fetches `/chat-transcript?session_id=...&project_dir=...` (skipped if messages already cached). `buildMessagesFromTranscript` converts the `.jsonl` transcript into `ChatMessage[]`.

---

## 8. CSRF Tokens

Every mutating request includes `csrf_token` in the JSON body. The server returns a rotated token in `x-csrf-token` on successful responses. `SseContext` stores the current token in a ref (`csrfTokenRef`) and updates it on each response, ensuring the next request always uses the latest value.

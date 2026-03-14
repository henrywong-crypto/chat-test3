import { type Page } from "@playwright/test";
import path from "path";

const DIST_DIR = path.join(__dirname, "../../dist");

export const VM_ID = "test-vm";
export const CSRF_TOKEN = "test-csrf";

// ── SSE types ─────────────────────────────────────────────────────────────

export interface Question {
  question: string;
  header?: string;
  options: { label: string; description?: string }[];
  multiSelect?: boolean;
}

export type SseEvent =
  | { event: "init" }
  | { event: "text_delta"; data: { text: string } }
  | { event: "thinking_delta"; data: { thinking: string } }
  | { event: "tool_start"; data: { id: string; name: string; input: Record<string, unknown> } }
  | { event: "tool_result"; data: { tool_use_id: string; content: string; is_error: boolean } }
  | { event: "ask_user_question"; data: { request_id: string; questions: Question[] } }
  | { event: "done"; data: { session_id: string | null } }
  | { event: "error_event"; data: { message: string } };

export function buildSseBody(events: SseEvent[]): string {
  return events
    .map((e) => {
      const data = "data" in e ? JSON.stringify(e.data) : "{}";
      return `event: ${e.event}\ndata: ${data}\n\n`;
    })
    .join("");
}

// Preset event sequences

export const sse = {
  text: (text: string, sessionId: string): SseEvent[] => [
    { event: "init" },
    { event: "text_delta", data: { text } },
    { event: "done", data: { session_id: sessionId } },
  ],

  // init → text_delta with no thinking_delta in between (tests empty indicator removal)
  noThinking: (text: string, sessionId: string): SseEvent[] => [
    { event: "init" },
    { event: "text_delta", data: { text } },
    { event: "done", data: { session_id: sessionId } },
  ],

  withThinking: (thinking: string, text: string, sessionId: string): SseEvent[] => [
    { event: "init" },
    { event: "thinking_delta", data: { thinking } },
    { event: "text_delta", data: { text } },
    { event: "done", data: { session_id: sessionId } },
  ],

  withTool: (
    toolId: string,
    toolName: string,
    input: Record<string, unknown>,
    result: string,
    text: string,
    sessionId: string,
  ): SseEvent[] => [
    { event: "init" },
    { event: "tool_start", data: { id: toolId, name: toolName, input } },
    { event: "tool_result", data: { tool_use_id: toolId, content: result, is_error: false } },
    { event: "text_delta", data: { text } },
    { event: "done", data: { session_id: sessionId } },
  ],

  question: (requestId: string, questions: Question[]): SseEvent[] => [
    { event: "init" },
    { event: "ask_user_question", data: { request_id: requestId, questions } },
    // done is sent separately after the user answers
  ],
};

// ── Mock session data ─────────────────────────────────────────────────────

export interface Session {
  session_id: string;
  created_at: string;
  title: string;
  project_dir: string;
}

export function makeSession(overrides: Partial<Session> = {}): Session {
  return {
    session_id: "sess-abc123",
    created_at: new Date().toISOString(),
    title: "hello",
    project_dir: "/home/ubuntu",
    ...overrides,
  };
}

// ── App HTML ──────────────────────────────────────────────────────────────

function buildAppHtml(): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>Web</title>
  <link rel="stylesheet" href="/static/styles.css"/>
</head>
<body class="flex h-screen overflow-hidden bg-background text-foreground">
  <div id="app-config" hidden
    data-vm-id="${VM_ID}"
    data-csrf-token="${CSRF_TOKEN}"
    data-upload-dir="/tmp"
    data-upload-action="/sessions/${VM_ID}/upload"
    data-has-user-rootfs="false"
  ></div>
  <div id="app" class="flex h-screen w-screen overflow-hidden"></div>
  <script src="/static/app.js" defer></script>
</body>
</html>`;
}

// ── App controller ────────────────────────────────────────────────────────

export interface AppController {
  /** Push SSE events through the currently open stream. */
  sendSseEvents(events: SseEvent[]): void;
  /** Replace the session list returned by subsequent /chat-history calls. */
  setSessions(sessions: Session[]): void;
  /** Body of the most recent POST /chat, or null. */
  lastChatBody(): { content: string; session_id: string | null } | null;
  /** Bodies of every POST /chat in order. */
  allChatBodies(): Array<{ content: string; session_id: string | null }>;
  /** Whether a stop request was received. */
  stopRequested(): boolean;
  /** Body of the most recent POST /chat-question-answer, or null. */
  lastAnswerBody(): { request_id: string; answers: Record<string, string> } | null;
}

export async function setupApp(
  page: Page,
  opts: { sessions?: Session[]; transcripts?: Record<string, unknown[]> } = {},
): Promise<AppController> {
  let sessions: Session[] = opts.sessions ?? [];
  const transcripts: Record<string, unknown[]> = opts.transcripts ?? {};

  const chatBodies: Array<{ content: string; session_id: string | null }> = [];
  let stopReceived = false;
  let lastAnswer: { request_id: string; answers: Record<string, string> } | null = null;

  // resolveSse is set each time the EventSource connects/reconnects.
  // sendSseEvents calls it to deliver events and close the stream.
  let resolveSse: ((events: SseEvent[]) => void) | null = null;

  // ── App HTML page ────────────────────────────────────────────────────────
  await page.route("http://localhost/", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: buildAppHtml() }),
  );

  // ── Static files ────────────────────────────────────────────────────────
  await page.route("**/static/app.js", (route) =>
    route.fulfill({ path: path.join(DIST_DIR, "app.js"), contentType: "application/javascript" }),
  );
  await page.route("**/static/styles.css", (route) =>
    route.fulfill({ path: path.join(DIST_DIR, "styles.css"), contentType: "text/css" }),
  );
  await page.route("**/favicon.ico", (route) => route.fulfill({ status: 204 }));

  // ── Session history ──────────────────────────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/chat-history`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(sessions),
    }),
  );

  // ── Transcript (GET) and delete (DELETE) ─────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/chat-transcript**`, async (route) => {
    if (route.request().method() === "DELETE") {
      const body = route.request().postDataJSON() as { session_id: string };
      sessions = sessions.filter((s) => s.session_id !== body.session_id);
      await route.fulfill({ status: 200, body: "" });
    } else {
      const url = new URL(route.request().url());
      const sessionId = url.searchParams.get("session_id") ?? "";
      const messages = transcripts[sessionId] ?? [];
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ messages }),
      });
    }
  });

  // ── File listing (for Files tab) ─────────────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/ls**`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ entries: [] }),
    }),
  );

  // ── Stop endpoint ────────────────────────────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/chat-stop`, async (route) => {
    stopReceived = true;
    await route.fulfill({ status: 200, body: "" });
  });

  // ── Question answer endpoint ──────────────────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/chat-question-answer`, async (route) => {
    lastAnswer = route.request().postDataJSON() as {
      request_id: string;
      answers: Record<string, string>;
    };
    await route.fulfill({ status: 200, body: "" });
  });

  // ── SSE stream — deferred until sendSseEvents is called ───────────────────
  // Each time the EventSource connects (initial or reconnect) the handler fires
  // and waits. Calling sendSseEvents resolves it with the desired events.
  await page.route(`**/sessions/${VM_ID}/chat-stream`, async (route) => {
    const events = await new Promise<SseEvent[]>((resolve) => {
      resolveSse = resolve;
    });
    resolveSse = null;
    await route.fulfill({
      status: 200,
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        "X-Accel-Buffering": "no",
      },
      body: buildSseBody(events),
    });
  });

  // ── Chat message endpoint ─────────────────────────────────────────────────
  await page.route(`**/sessions/${VM_ID}/chat`, async (route) => {
    const body = route.request().postDataJSON() as {
      content: string;
      session_id: string | null;
    };
    chatBodies.push(body);
    await route.fulfill({ status: 200, body: "" });
  });

  // ── Load the app ──────────────────────────────────────────────────────────
  // Navigate to a routed URL so that script src="/static/app.js" resolves correctly.
  await page.goto("http://localhost/", { waitUntil: "domcontentloaded" });
  // Wait for React to render the composer — by this point all useEffects have run
  // (including the SseProvider effect that opens the EventSource).
  await page.waitForSelector('textarea[placeholder="Message Claude…"]');

  return {
    sendSseEvents: (events) => resolveSse?.(events),
    setSessions: (s) => {
      sessions = s;
    },
    lastChatBody: () => chatBodies[chatBodies.length - 1] ?? null,
    allChatBodies: () => [...chatBodies],
    stopRequested: () => stopReceived,
    lastAnswerBody: () => lastAnswer,
  };
}

// ── Test interaction helpers ───────────────────────────────────────────────

/** Fill the composer and submit with Enter. */
export async function sendMessage(page: Page, text: string): Promise<void> {
  await page.getByPlaceholder("Message Claude…").fill(text);
  await page.keyboard.press("Enter");
}

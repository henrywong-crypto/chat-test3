/**
 * UF-34  New Chat mid-stream (null session)    — status bar stays visible after clicking New Chat
 * UF-35  Done fires after stale New Chat       — status bar clears, view stays blank
 * UF-36  New session in sidebar after stale done — session appears but view stays blank
 * UF-37  New Chat while streaming existing session — status bar disappears immediately
 * UF-38  Sidebar pulsing indicator stays        — shown on running session after navigating away
 * UF-39  Navigate back to running session       — status bar reappears
 * UF-40  Done fires while viewing new chat      — sidebar indicator clears, view unchanged
 * UF-41  Non-running session has active composer — can type when viewing a different session
 * UF-42  Click existing session mid-stream (null) — status bar disappears
 * UF-43  New Chat hides thinking indicator      — animated dots disappear when navigating away
 * UF-44  Navigate back shows thinking indicator — animated dots reappear when returning to running session
 */
import { test, expect } from "@playwright/test";
import { setupApp, sendMessage, sse, makeSession } from "./helpers/setup";

test.describe("navigation during streaming", () => {
  // ── Null-session streaming ─────────────────────────────────────────────────

  test("UF-34 clicking New Chat while a null-session stream is pending keeps the status bar visible", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    // isStreaming=true, runningSessionId=null, viewSessionId=null → isLoading=true
    await expect(page.getByRole("status")).toBeVisible();

    // Click New Chat before any SSE events arrive
    await page.getByRole("button", { name: "New Chat" }).click();

    // viewSessionId stays null, runningSessionId stays null → isLoading still true
    await expect(page.getByRole("status")).toBeVisible();

    // Clean up so the stream doesn't leak into the next test
    ctrl.sendSseEvents([{ event: "done", data: { session_id: null } }]);
    await expect(page.getByRole("status")).not.toBeVisible();
  });

  test("UF-35 status bar clears when done fires after a stale New Chat click", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    await page.getByRole("button", { name: "New Chat" }).click();
    // Status bar is still visible at this point (UF-34)
    await expect(page.getByRole("status")).toBeVisible();

    ctrl.sendSseEvents(sse.text("The response", "sess-new"));

    // done fires → isStreaming=false → isLoading=false → status bar gone
    await expect(page.getByRole("status")).not.toBeVisible();
    // stale-message detection discards the response; blank state remains
    await expect(page.getByText("Start a new conversation")).toBeVisible();
  });

  test("UF-36 new session appears in the sidebar even when New Chat was clicked mid-stream", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    await page.getByRole("button", { name: "New Chat" }).click();

    // Pre-populate history so the refresh triggered by done returns the new session
    ctrl.setSessions([makeSession({ session_id: "sess-new", title: "My stale session" })]);
    ctrl.sendSseEvents(sse.text("The response", "sess-new"));

    // New session surfaces in the sidebar from the history refresh
    await expect(
      page.locator("span.truncate").filter({ hasText: "My stale session" }),
    ).toBeVisible();
    // But the current view is still blank — no navigation to the stale session
    await expect(page.getByText("Start a new conversation")).toBeVisible();
  });

  // ── Existing-session streaming ────────────────────────────────────────────

  test("UF-37 clicking New Chat while streaming an existing session immediately hides the status bar", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Old Chat" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Old Chat").click();
    await sendMessage(page, "Hello from sess-a");
    // isStreaming=true, runningSessionId="sess-a", viewSessionId="sess-a" → isLoading=true
    await expect(page.getByRole("status")).toBeVisible();

    await page.getByRole("button", { name: "New Chat" }).click();
    // viewSessionId=null, runningSessionId="sess-a" → isLoading=false
    await expect(page.getByRole("status")).not.toBeVisible();
  });

  test("UF-38 sidebar pulsing indicator stays on the running session after navigating to new chat", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Running Session" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Running Session").click();
    await sendMessage(page, "Long task");
    await page.getByRole("button", { name: "New Chat" }).click();

    // Status bar is gone (different session context) but the sidebar dot still pulses
    await expect(page.getByRole("status")).not.toBeVisible();
    await expect(page.locator(".animate-ping")).toBeVisible();
  });

  test("UF-39 navigating back to the running session restores the status bar", async ({ page }) => {
    const session = makeSession({ session_id: "sess-a", title: "Running Session" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Running Session").click();
    await sendMessage(page, "Long task");
    await page.getByRole("button", { name: "New Chat" }).click();
    await expect(page.getByRole("status")).not.toBeVisible();

    // Click the session row to return
    await page.getByText("Running Session").click();
    // runningSessionId="sess-a" === viewSessionId="sess-a" → isLoading=true again
    await expect(page.getByRole("status")).toBeVisible();
  });

  test("UF-40 done fires while viewing new chat — sidebar indicator clears, view unchanged", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Running Session" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Running Session").click();
    await sendMessage(page, "Long task");
    await page.getByRole("button", { name: "New Chat" }).click();
    // Wait for the status bar to disappear before checking the sidebar dot —
    // otherwise the ClaudeStatus animate-ping dot is still in the DOM (useEffect lag)
    await expect(page.getByRole("status")).not.toBeVisible();
    await expect(page.locator(".animate-ping")).toBeVisible();

    // Complete the stream while the user is on the new-chat view
    ctrl.setSessions([session]);
    ctrl.sendSseEvents(sse.text("Finished.", "sess-a"));

    // Sidebar pulsing indicator clears (runningSessionId set to null)
    await expect(page.locator(".animate-ping")).not.toBeVisible();
    // Status bar stays hidden (we are not viewing sess-a)
    await expect(page.getByRole("status")).not.toBeVisible();
    // The new-chat blank state is unchanged
    await expect(page.getByText("Start a new conversation")).toBeVisible();
  });

  // ── Cross-session navigation ───────────────────────────────────────────────

  test("UF-41 viewing a non-running session gives an active composer during streaming", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Other Chat" });
    const ctrl = await setupApp(page, { sessions: [session] });

    // Start streaming from the new-chat blank view
    await sendMessage(page, "Streaming from new chat");
    await expect(page.getByRole("status")).toBeVisible();

    // Switch to an existing session — it is not the running one
    await page.getByText("Other Chat").click();

    // isLoading=false for this session → no status bar, composer is active
    await expect(page.getByRole("status")).not.toBeVisible();
    await expect(page.getByPlaceholder("Message Claude…")).toBeEnabled();
  });

  test("UF-42 clicking an existing session while a null-session stream is pending hides the status bar", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Previous Chat" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await sendMessage(page, "Hello");
    // null-session stream: runningSessionId=null, viewSessionId=null → isLoading=true
    await expect(page.getByRole("status")).toBeVisible();

    // Navigate to the existing session
    await page.getByText("Previous Chat").click();
    // viewSessionId="sess-a", runningSessionId=null → isLoading=false
    await expect(page.getByRole("status")).not.toBeVisible();

    // No pulsing indicator either (runningSessionId=null matches no session row)
    await expect(page.locator(".animate-ping")).not.toBeVisible();
  });

  // ── Thinking indicator visibility ─────────────────────────────────────────

  test("UF-43 clicking New Chat while the thinking indicator is visible hides it", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Running Session" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Running Session").click();
    await sendMessage(page, "Long task");
    // Wait for the streaming render to commit so runningSessionId="sess-a" is in the ref
    await expect(page.getByRole("status")).toBeVisible();

    // init fires → thinking indicator added to sess-a's messages
    ctrl.sendSseEvents([{ event: "init" }]);
    await expect(page.locator(".thinking-dot").first()).toBeVisible();

    // Navigate to New Chat — sess-a's messages are no longer shown
    await page.getByRole("button", { name: "New Chat" }).click();
    await expect(page.locator(".thinking-dot").first()).not.toBeVisible();
  });

  test("UF-44 navigating back to the running session restores the thinking indicator", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-a", title: "Running Session" });
    const ctrl = await setupApp(page, { sessions: [session] });

    await page.getByText("Running Session").click();
    await sendMessage(page, "Long task");
    // Wait for the streaming render to commit so runningSessionId="sess-a" is in the ref
    await expect(page.getByRole("status")).toBeVisible();

    // init fires → thinking indicator added to sess-a's messages
    ctrl.sendSseEvents([{ event: "init" }]);
    await expect(page.locator(".thinking-dot").first()).toBeVisible();

    // Navigate away — thinking indicator hidden
    await page.getByRole("button", { name: "New Chat" }).click();
    await expect(page.locator(".thinking-dot").first()).not.toBeVisible();

    // Return to the running session — thinking indicator reappears
    await page.getByText("Running Session").click();
    await expect(page.locator(".thinking-dot").first()).toBeVisible();
  });
});

/**
 * UF-01  Blank new chat      — "Start a new conversation" shown when no sessions
 * UF-02  Send message        — user bubble + status bar appear
 * UF-03  Receive response    — assistant message shown, status bar gone, session in sidebar
 * UF-04  New Chat button     — clears to blank state
 * UF-05  New Chat + streaming — stays blank after done fires (not navigated away)
 */
import { test, expect } from "@playwright/test";
import { setupApp, sendMessage, makeSession, sse } from "./helpers/setup";

test.describe("chat", () => {
  test("UF-01 blank state shown on load with no sessions", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    await expect(page.getByText("Start a new conversation")).toBeVisible();
    await expect(page.getByText("No conversations yet")).toBeVisible();
  });

  test("UF-02 sending a message shows user bubble and streaming status bar", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello Claude");

    // User's message bubble is immediately visible
    await expect(page.getByText("Hello Claude")).toBeVisible();

    // ClaudeStatus bar appears while streaming (has role=status)
    await expect(page.getByRole("status")).toBeVisible();
  });

  test("UF-03 receiving response shows assistant message and session in sidebar", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hi");

    // Update the sessions list BEFORE sending done so that loadHistory()
    // (triggered by the done event) picks up the new session.
    ctrl.setSessions([makeSession({ session_id: "sess-1", title: "Hi" })]);
    ctrl.sendSseEvents(sse.text("Hello! How can I help?", "sess-1"));

    await expect(page.getByText("Hello! How can I help?")).toBeVisible();
    await expect(page.getByRole("status")).not.toBeVisible();
    // Session title "Hi" appears in the sidebar
    await expect(page.locator("span.truncate").filter({ hasText: /^Hi$/ })).toBeVisible();
  });

  test("UF-04 New Chat button resets to blank state", async ({ page }) => {
    const ctrl = await setupApp(page, {
      sessions: [makeSession({ session_id: "sess-1", title: "hello" })],
    });

    // Click the existing session so we're viewing it
    await page.getByText("hello").click();
    // Now click New Chat
    await page.getByRole("button", { name: "New Chat" }).click();

    await expect(page.getByText("Start a new conversation")).toBeVisible();
    // No session highlighted in sidebar
    const activeRow = page.locator(".border-l-2.border-primary");
    await expect(activeRow).not.toBeVisible();
  });

  test("UF-05 clicking New Chat while streaming stays blank after done fires", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    // 1. Send a message — stream is now "in flight" (no events sent yet)
    await sendMessage(page, "Hello");

    // 2. Click New Chat before the stream resolves
    await page.getByRole("button", { name: "New Chat" }).click();

    // 3. Now resolve the SSE stream (simulates done arriving after New Chat was clicked)
    ctrl.setSessions([makeSession({ session_id: "sess-2", title: "Hello" })]);
    ctrl.sendSseEvents(sse.text("Hi!", "sess-2"));

    // 4. The chat should remain blank — not navigated to the completed session
    await expect(page.getByText("Start a new conversation")).toBeVisible();
  });
});

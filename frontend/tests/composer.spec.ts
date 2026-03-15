/**
 * UF-30  Shift+Enter        — inserts newline in composer without submitting
 * UF-31  SSE error event    — error message shown in chat, streaming cleared
 * UF-32  Copy button hover  — hovering an assistant message reveals copy button
 * UF-33  Copy format picker — format dropdown shows markdown / text options
 * UF-56  New Chat auto-focus — clicking "New Chat" puts focus on the composer textarea
 * UF-57  Post-send focus    — after sending a message the composer textarea regains focus
 * UF-58  Session select focus — switching to a history session focuses the composer textarea
 */
import { test, expect } from "@playwright/test";
import { setupApp, sendMessage, sse, makeSession } from "./helpers/setup";

test.describe("composer", () => {
  test("UF-30 Shift+Enter inserts a newline without submitting", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    const composer = page.getByPlaceholder("Message Claude…");
    await composer.click();
    await composer.type("line1");
    await composer.press("Shift+Enter");
    await composer.type("line2");

    // No status bar — message was not submitted
    await expect(page.getByRole("status")).not.toBeVisible();

    // Composer contains a newline separating the two lines
    const value = await composer.inputValue();
    expect(value).toBe("line1\nline2");
  });

  test("UF-31 SSE error event shows an error message in the chat", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Do something");
    ctrl.sendSseEvents([
      { event: "init" },
      { event: "error_event", data: { message: "Something went wrong on the server" } },
    ]);

    // Error text appears in the chat
    await expect(page.getByText("Something went wrong on the server")).toBeVisible();
    // Streaming state is cleared — status bar gone
    await expect(page.getByRole("status")).not.toBeVisible();
  });

  test("UF-32 hovering an assistant message reveals the copy button", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    ctrl.sendSseEvents(sse.text("Here is my response.", "sess-1"));

    await expect(page.getByText("Here is my response.")).toBeVisible();

    // Hover over the message text — the copy button becomes visible
    await page.getByText("Here is my response.").hover();
    await expect(page.getByTitle("Copy", { exact: true })).toBeVisible();
  });

  test("UF-33 copy format dropdown shows markdown and plain-text options", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    ctrl.sendSseEvents(sse.text("**Bold** response", "sess-1"));

    await expect(page.getByText("Bold").first()).toBeVisible();

    // Hover to reveal copy controls then open the format picker
    await page.getByText("Bold").first().hover();
    await page.getByTitle("Select copy format").click();

    await expect(page.getByText("Copy as markdown")).toBeVisible();
    await expect(page.getByText("Copy as text")).toBeVisible();

    // Switching to text format updates the button label
    await page.getByText("Copy as text").click();
    await expect(page.getByTitle("Copy", { exact: true })).toBeVisible();
    // Format label on the button switches to TXT
    await expect(page.locator("button[title='Copy'] span").filter({ hasText: "TXT" })).toBeVisible();
  });

  test("UF-56 clicking New Chat focuses the composer textarea", async ({ page }) => {
    const session = makeSession({ session_id: "sess-abc", title: "Old chat" });
    await setupApp(page, { sessions: [session] });

    // Navigate away to an existing session
    await page.getByText("Old chat").click();

    // Click "New Chat" in the sidebar
    await page.getByRole("button", { name: "New Chat" }).click();

    // The textarea should have focus immediately
    const composer = page.getByPlaceholder("Message Claude…");
    await expect(composer).toBeFocused();
  });

  test("UF-57 composer textarea regains focus after sending a message", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");

    // Complete the stream so loading ends and the composer re-enables
    ctrl.sendSseEvents(sse.text("Hi there", "sess-1"));
    await expect(page.getByRole("status")).not.toBeVisible();

    const composer = page.getByPlaceholder("Message Claude…");
    await expect(composer).toBeFocused();
  });

  test("UF-58 switching to a history session focuses the composer textarea", async ({ page }) => {
    const session = makeSession({ session_id: "sess-abc", title: "Past chat" });
    await setupApp(page, { sessions: [session] });

    await page.getByText("Past chat").click();

    const composer = page.getByPlaceholder("Message Claude…");
    await expect(composer).toBeFocused();
  });
});

/**
 * PS-01  Placeholder on send        — "New chat…" row appears in sidebar immediately after pressing Enter
 * PS-02  New Chat while streaming   — placeholder stays in sidebar with pulsing indicator; composer blank
 * PS-03  done replaces placeholder  — loadHistory response swaps placeholder for real session title
 * PS-04  Click placeholder          — navigates back to show in-progress messages
 * PS-05  POST /chat error           — placeholder never shown, error rendered, composer usable
 */
import { test, expect } from "@playwright/test";
import { setupApp, sendMessage, makeSession, sse } from "./helpers/setup";

test.describe("pending-session", () => {
  test("PS-01 placeholder appears in sidebar immediately on send", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    // Send but do NOT resolve SSE yet — placeholder must appear before any events arrive
    await sendMessage(page, "Hello Claude");

    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).toBeVisible();
  });

  test("PS-02 clicking New Chat while streaming keeps placeholder with pulsing indicator", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello Claude");

    // Placeholder visible immediately
    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).toBeVisible();

    // Click New Chat mid-stream
    await page.getByRole("button", { name: "New Chat" }).click();

    // Blank state shown (new chat view)
    await expect(page.getByText("Start a new conversation")).toBeVisible();

    // Placeholder still in sidebar
    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).toBeVisible();

    // Pulsing indicator visible on the placeholder row
    await expect(page.locator(".animate-ping")).toBeVisible();
  });

  test("PS-03 done event replaces placeholder with real session title", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");

    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).toBeVisible();

    ctrl.setSessions([makeSession({ session_id: "sess-1", title: "Hello session" })]);
    ctrl.sendSseEvents(sse.text("Hi there!", "sess-1"));

    // Real session title should replace the placeholder
    await expect(page.locator("span.truncate").filter({ hasText: "Hello session" })).toBeVisible();
    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).not.toBeVisible();
  });

  test("PS-04 clicking placeholder while streaming shows in-progress messages", async ({ page }) => {
    await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello Claude");

    // Switch away via New Chat
    await page.getByRole("button", { name: "New Chat" }).click();
    await expect(page.getByText("Start a new conversation")).toBeVisible();

    // Click the placeholder row to navigate back
    await page.locator("span.truncate").filter({ hasText: "New chat\u2026" }).click();

    // Original message should be visible again
    await expect(page.getByText("Hello Claude")).toBeVisible();
  });

  test("PS-05 POST /chat error: placeholder removed, error shown, composer usable", async ({ page }) => {
    await setupApp(page, { sessions: [], chatError: "Service unavailable" });

    await sendMessage(page, "Hello");

    // Wait for the error to appear — at that point the placeholder is already removed
    await expect(page.getByText("Service unavailable")).toBeVisible();

    // Placeholder should not be in the sidebar
    await expect(page.locator("span.truncate").filter({ hasText: "New chat\u2026" })).not.toBeVisible();

    // Composer should be usable (not disabled)
    await expect(page.getByPlaceholder("Message Claude\u2026")).toBeEnabled();
  });
});

/**
 * UF-11  Resume session       — clicking session in sidebar loads its transcript
 * UF-12  Tool results resume  — tool cards from transcript include result
 * UF-13  Delete session       — hovering + clicking trash removes session
 * UF-14  Refresh sessions     — refresh button re-fetches history
 */
import { test, expect } from "@playwright/test";
import { setupApp, makeSession } from "./helpers/setup";

test.describe("session", () => {
  test("UF-11 clicking a session in the sidebar loads its transcript", async ({ page }) => {
    const session = makeSession({ session_id: "sess-abc", title: "my chat" });

    await setupApp(page, {
      sessions: [session],
      transcripts: {
        "sess-abc": [
          {
            role: "user",
            content: [{ type: "text", text: "What is 2+2?" }],
            isCompactSummary: false,
          },
          {
            role: "assistant",
            content: [{ type: "text", text: "It is 4." }],
            isCompactSummary: false,
          },
        ],
      },
    });

    // Session appears in sidebar; click it
    await page.getByText("my chat").click();

    // Transcript messages are shown
    await expect(page.getByText("What is 2+2?")).toBeVisible();
    await expect(page.getByText("It is 4.")).toBeVisible();

    // Session row is highlighted
    const activeRow = page.locator(".border-l-2.border-primary");
    await expect(activeRow).toBeVisible();
  });

  test("UF-12 tool results are shown when resuming a session from transcript", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-tool", title: "tool session" });

    await setupApp(page, {
      sessions: [session],
      transcripts: {
        "sess-tool": [
          {
            role: "assistant",
            content: [
              {
                type: "tool_use",
                id: "call-1",
                name: "Bash",
                input: { command: "echo hello" },
              },
            ],
            isCompactSummary: false,
          },
          {
            role: "user",
            content: [
              {
                type: "tool_result",
                tool_use_id: "call-1",
                content: "hello",
                is_error: false,
              },
            ],
            isCompactSummary: false,
          },
          {
            role: "assistant",
            content: [{ type: "text", text: "All done." }],
            isCompactSummary: false,
          },
        ],
      },
    });

    await page.getByText("tool session").click();

    // Tool card is visible with its name
    await expect(page.getByText("Bash")).toBeVisible();
    // Tool result is shown in the card
    await expect(page.getByText("hello")).toBeVisible();
    // Follow-up text visible
    await expect(page.getByText("All done.")).toBeVisible();
  });

  test("UF-13 hovering a session and clicking delete removes it from the list", async ({
    page,
  }) => {
    const session = makeSession({ session_id: "sess-del", title: "to be deleted" });

    await setupApp(page, { sessions: [session] });

    // Reveal the delete button by hovering the session row
    await page.locator(".group").filter({ hasText: "to be deleted" }).hover();

    // Click the trash icon button inside the row
    await page
      .locator(".group")
      .filter({ hasText: "to be deleted" })
      .locator("button")
      .click();

    // Session is no longer visible in the sidebar
    await expect(page.getByText("to be deleted")).not.toBeVisible();
    await expect(page.getByText("No conversations yet")).toBeVisible();
  });

  test("UF-14 refresh button re-fetches the session list", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    // Sidebar shows empty state
    await expect(page.getByText("No conversations yet")).toBeVisible();

    // Inject a session server-side before clicking refresh
    ctrl.setSessions([makeSession({ session_id: "sess-new", title: "new session" })]);

    // Click the circular refresh icon in the sidebar header
    await page.getByTitle("Refresh").click();

    // New session should now appear
    await expect(page.getByText("new session")).toBeVisible();
  });
});

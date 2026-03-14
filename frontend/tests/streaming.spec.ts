/**
 * UF-06  Empty thinking removed  — animated dots gone when no thinking_delta arrives
 * UF-07  Thinking block shown    — collapsible "Thinking…" when thinking content present
 * UF-08  Tool use with result    — tool card shows result after tool_result event
 * UF-09  Stop streaming          — Stop button sends stop request
 * UF-10  Ask user question       — panel shown; selecting an option and submitting works
 */
import { test, expect } from "@playwright/test";
import { setupApp, sendMessage, sse } from "./helpers/setup";

test.describe("streaming", () => {
  test("UF-06 empty thinking indicator is removed when no thinking content arrives", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Hello");
    ctrl.sendSseEvents(sse.noThinking("Reply without thinking", "sess-1"));

    await expect(page.getByText("Reply without thinking")).toBeVisible();

    // The animated ThinkingIndicator renders only when isThinking=true AND content=""
    // Our fix removes it on text_delta; assert it is NOT in the DOM.
    const thinkingIndicator = page.locator(".thinking-dot").first();
    await expect(thinkingIndicator).not.toBeVisible();
  });

  test("UF-07 thinking block shown as collapsible when thinking content is present", async ({
    page,
  }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Deep question");
    ctrl.sendSseEvents(sse.withThinking("My reasoning here…", "The answer is 42.", "sess-2"));

    // The collapsible <details> summary shows "Thinking…"
    await expect(page.getByText("Thinking…")).toBeVisible();
    // The assistant response is also shown
    await expect(page.getByText("The answer is 42.")).toBeVisible();
  });

  test("UF-08 tool card shown with result after tool_result event", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Run ls");
    ctrl.sendSseEvents(
      sse.withTool(
        "tool-1",
        "Bash",
        { command: "ls" },
        "file1.txt\nfile2.txt",
        "Done.",
        "sess-3",
      ),
    );

    // Tool name visible in the tool card
    await expect(page.getByText("Bash")).toBeVisible();
    // Click to expand the tool card and see the result
    await page.getByRole("button", { name: /Bash/ }).click();
    // Tool result visible inside the expanded card
    await expect(page.getByText("file1.txt")).toBeVisible();
    // Assistant follow-up text
    await expect(page.getByText("Done.")).toBeVisible();
  });

  test("UF-09 clicking Stop sends a stop request to the server", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    // Send a message so the streaming state activates (no SSE events yet)
    await sendMessage(page, "Long task");

    // The stop button is in the ClaudeStatus bar while streaming
    await expect(page.getByRole("status")).toBeVisible();
    await page.getByTitle("Stop (Esc)").first().click();

    expect(ctrl.stopRequested()).toBe(true);
  });

  test("UF-10 ask user question panel shown and answer submitted", async ({ page }) => {
    const ctrl = await setupApp(page, { sessions: [] });

    await sendMessage(page, "Help me choose");
    ctrl.sendSseEvents(
      sse.question("req-1", [
        {
          question: "Which option do you prefer?",
          header: "Preference",
          options: [
            { label: "Option A", description: "First choice" },
            { label: "Option B", description: "Second choice" },
          ],
        },
      ]),
    );

    // Panel header is visible
    await expect(page.getByText("Claude needs your input")).toBeVisible();
    await expect(page.getByText("Which option do you prefer?")).toBeVisible();

    // Click the first option
    await page.getByRole("button", { name: "Option A" }).click();

    // Submit the answer
    await page.getByRole("button", { name: "Submit" }).click();

    // The answer request should have been sent
    const answerBody = ctrl.lastAnswerBody();
    expect(answerBody?.request_id).toBe("req-1");
    expect(answerBody?.answers["Which option do you prefer?"]).toContain("Option A");
  });
});

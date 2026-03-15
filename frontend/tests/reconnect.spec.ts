/**
 * RC-01  Chat-hello sent on SSE reconnect with saved task_id
 * RC-02  Question panel restored after reconnect
 * RC-03  Done event after reconnect clears running state
 * RC-04  Messages restored from localStorage on reconnect
 *
 * These tests use page.addInitScript to pre-set localStorage before the app
 * loads, simulating a previous session that was interrupted. The first SSE
 * connection's onopen handler then sees the saved task_id and sends chat-hello.
 */
import { test, expect } from "@playwright/test";
import { setupApp, VM_ID } from "./helpers/setup";

const TASK_ID = "task-reconnect-test";

test.describe("reconnect", () => {
  test("RC-01 chat-hello sent on SSE reconnect with saved task_id", async ({ page }) => {
    await page.addInitScript((vmId) => {
      localStorage.setItem(
        `chat_running_task_${vmId}`,
        JSON.stringify({ task_id: "task-rc01", running_session_id: null }),
      );
    }, VM_ID);

    const ctrl = await setupApp(page, { sessions: [] });

    // Fulfill the first SSE stream — this triggers onopen which reads localStorage
    // and POSTs chat-hello, then emits a reconnecting event.
    const helloResponsePromise = page.waitForResponse(`**/sessions/${VM_ID}/chat-hello`);
    ctrl.sendSseEvents([{ event: "done", data: { session_id: null, task_id: "task-rc01" } }]);

    await helloResponsePromise;
    expect(ctrl.lastHelloBody()?.task_id).toBe("task-rc01");
  });

  test("RC-02 question panel restored after reconnect", async ({ page }) => {
    await page.addInitScript((vmId) => {
      localStorage.setItem(
        `chat_running_task_${vmId}`,
        JSON.stringify({ task_id: "task-rc02", running_session_id: null }),
      );
    }, VM_ID);

    const ctrl = await setupApp(page, { sessions: [] });

    // Send ask_user_question as the first SSE event — onopen fires first (reconnecting),
    // then the question event is processed and the panel is shown.
    ctrl.sendSseEvents([
      {
        event: "ask_user_question",
        data: {
          request_id: "req-rc02",
          task_id: "task-rc02",
          questions: [{ question: "Pick one?", options: [{ label: "A" }, { label: "B" }] }],
        },
      },
    ]);

    await expect(page.getByText("Pick one?")).toBeVisible();

    // Clean up
    ctrl.sendSseEvents([{ event: "done", data: { session_id: null, task_id: "task-rc02" } }]);
  });

  test("RC-03 done event after reconnect clears running state", async ({ page }) => {
    await page.addInitScript((vmId) => {
      localStorage.setItem(
        `chat_running_task_${vmId}`,
        JSON.stringify({ task_id: "task-rc03", running_session_id: null }),
      );
    }, VM_ID);

    const ctrl = await setupApp(page, { sessions: [] });

    // The reconnecting event sets isStreaming=true, so the status bar should appear.
    // Then we send done to clear the running state.
    ctrl.sendSseEvents([{ event: "done", data: { session_id: null, task_id: "task-rc03" } }]);

    await expect(page.getByRole("status")).not.toBeVisible();

    // localStorage entry for running task should be cleared
    const storedTask = await page.evaluate(
      (vmId) => localStorage.getItem(`chat_running_task_${vmId}`),
      VM_ID,
    );
    expect(storedTask).toBeNull();
  });

  test("RC-04 messages restored from localStorage on reconnect", async ({ page }) => {
    const savedMessages = JSON.stringify([
      { id: "msg-1", type: "user", content: "Hello", timestamp: Date.now() },
      { id: "msg-2", type: "assistant", content: "Prior response", timestamp: Date.now() },
    ]);

    await page.addInitScript(
      (args) => {
        const { vmId, taskId, messages } = args;
        localStorage.setItem(
          `chat_running_task_${vmId}`,
          JSON.stringify({ task_id: taskId, running_session_id: null }),
        );
        localStorage.setItem(`chat_messages_task_${taskId}`, messages);
      },
      { vmId: VM_ID, taskId: "task-rc04", messages: savedMessages },
    );

    const ctrl = await setupApp(page, { sessions: [] });

    // Trigger onopen → reconnecting event restores messages from localStorage
    ctrl.sendSseEvents([{ event: "done", data: { session_id: null, task_id: "task-rc04" } }]);

    // Prior messages should be visible (restored from localStorage before SSE events arrive)
    await expect(page.getByText("Prior response")).toBeVisible();
  });
});

import { useEffect, useRef } from "react";
import type { SseEvent } from "../types";
import type { ChatStateResult } from "./useChatState";

interface SseHandlerDeps {
  latestEvent: SseEvent | null;
  loadHistory: () => Promise<import("../types").ChatSession[]>;
  newChatKeyRef: { current: number };
  sessionStartKeyRef: { current: number };
  vmId: string;
}

export function useSseHandlers(
  sseState: SseHandlerDeps,
  chatState: ChatStateResult & { setSessions: (s: import("../types").ChatSession[]) => void },
) {
  const { latestEvent, loadHistory, newChatKeyRef, sessionStartKeyRef, vmId } = sseState;
  const {
    viewSessionId,
    runningSessionId,
    setRunningSessionId,
    setIsStreaming,
    setNullOrphaned,
    setSessionPendingQuestion,
    setTaskId,
    setSessions,
    addMessage,
    removeMessage,
    updateLastMessage,
    updateMessageById,
    getMessages,
    setMessages,
    setViewSessionId,
    generateId,
  } = chatState;

  // Track the current running session at event time via ref
  const runningRef = useRef(runningSessionId);
  runningRef.current = runningSessionId;

  const viewRef = useRef(viewSessionId);
  viewRef.current = viewSessionId;

  const nullOrphanedRef = useRef(chatState.nullOrphaned);
  nullOrphanedRef.current = chatState.nullOrphaned;

  // Track pending tool message IDs by tool_use_id
  const toolIdToMsgId = useRef<Map<string, string>>(new Map());

  // Track current thinking message id
  const thinkingMsgId = useRef<string | null>(null);

  // Track current assistant message id (accumulating text)
  const assistantMsgId = useRef<string | null>(null);

  useEffect(() => {
    if (!latestEvent) return;
    const event = latestEvent as SseEvent;
    const session = runningRef.current;

    // Seal the active thinking message. If it accumulated no content (no
    // thinking_delta arrived), remove it entirely so the animated dots don't
    // linger after the response starts arriving.
    const sealThinking = () => {
      if (!thinkingMsgId.current) return;
      const msgId = thinkingMsgId.current;
      thinkingMsgId.current = null;
      const msgs = getMessages(session);
      const thinkMsg = msgs.find((m) => m.id === msgId);
      if (thinkMsg && !thinkMsg.content) {
        removeMessage(session, msgId);
      }
    };

    switch (event.type) {
      case "session_start": {
        const { task_id } = event.payload;
        setTaskId(session, task_id);
        localStorage.setItem(
          `chat_running_task_${vmId}`,
          JSON.stringify({ task_id, running_session_id: session }),
        );
        break;
      }

      case "init": {
        // Push a thinking indicator as an assistant message
        const id = generateId();
        thinkingMsgId.current = id;
        assistantMsgId.current = null;
        addMessage(session, {
          id,
          type: "assistant",
          content: "",
          timestamp: Date.now(),
          isThinking: true,
        });
        break;
      }

      case "thinking_delta": {
        const { thinking } = event.payload;
        if (thinkingMsgId.current) {
          updateMessageById(session, thinkingMsgId.current, (m) => ({
            ...m,
            content: m.content + thinking,
          }));
        }
        break;
      }

      case "text_delta": {
        const { text } = event.payload;
        sealThinking();
        if (!assistantMsgId.current) {
          const id = generateId();
          assistantMsgId.current = id;
          addMessage(session, {
            id,
            type: "assistant",
            content: text,
            timestamp: Date.now(),
          });
        } else {
          updateMessageById(session, assistantMsgId.current, (m) => ({
            ...m,
            content: m.content + text,
          }));
        }
        break;
      }

      case "tool_start": {
        const { id: toolId, name, input } = event.payload;
        sealThinking();
        assistantMsgId.current = null;
        if (name === "AskUserQuestion") break;
        const msgId = generateId();
        toolIdToMsgId.current.set(toolId, msgId);
        addMessage(session, {
          id: msgId,
          type: "tool",
          content: "",
          timestamp: Date.now(),
          isToolUse: true,
          toolId,
          toolName: name,
          toolInput: input,
        });
        break;
      }

      case "tool_result": {
        const { tool_use_id, content, is_error } = event.payload;
        const msgId = toolIdToMsgId.current.get(tool_use_id);
        if (msgId) {
          updateMessageById(session, msgId, (m) => ({
            ...m,
            toolResult: { content, isError: is_error },
          }));
        }
        break;
      }

      case "ask_user_question": {
        const { request_id, task_id, questions } = event.payload;
        sealThinking();
        assistantMsgId.current = null;
        setSessionPendingQuestion(session, { requestId: request_id, taskId: task_id, questions });
        break;
      }

      case "done": {
        const { session_id } = event.payload;
        const completedSession = runningRef.current;
        setRunningSessionId(null);
        setIsStreaming(false);
        setNullOrphaned(false);
        setSessionPendingQuestion(completedSession, null);
        sealThinking();
        assistantMsgId.current = null;
        toolIdToMsgId.current.clear();

        if (session_id && completedSession === viewRef.current) {
          const newChatKeyUnchanged = completedSession !== null || sessionStartKeyRef.current === newChatKeyRef.current;
          if (newChatKeyUnchanged) {
            const msgs = getMessages(completedSession);
            setMessages(session_id, msgs);
            if (completedSession === null) {
              setMessages(null, []);
            }
            setViewSessionId(session_id);
          } else if (completedSession === null) {
            // New Chat was clicked after this session started — discard stale messages
            setMessages(null, []);
          }
        }

        loadHistory().then(setSessions).catch(console.error);
        break;
      }

      case "error_event": {
        const { message } = event.payload;
        const wasOrphaned = nullOrphanedRef.current;
        setRunningSessionId(null);
        setIsStreaming(false);
        setNullOrphaned(false);
        setSessionPendingQuestion(session, null);
        thinkingMsgId.current = null;
        assistantMsgId.current = null;
        toolIdToMsgId.current.clear();
        if (wasOrphaned) {
          setMessages(session, []);
        } else {
          addMessage(session, {
            id: generateId(),
            type: "error",
            content: message,
            timestamp: Date.now(),
          });
        }
        break;
      }

      case "reconnecting": {
        const { task_id, running_session_id } = event.payload;
        setTaskId(running_session_id, task_id);
        setRunningSessionId(running_session_id);
        setIsStreaming(true);
        setViewSessionId(running_session_id);
        break;
      }
    }
  }, [latestEvent]); // eslint-disable-line react-hooks/exhaustive-deps
}

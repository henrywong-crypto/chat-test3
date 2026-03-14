import { useEffect, useRef } from "react";
import type { SseEvent } from "../types";
import type { ChatStateResult } from "./useChatState";

interface SseHandlerDeps {
  latestEvent: SseEvent | null;
  loadHistory: () => Promise<import("../types").ChatSession[]>;
}

export function useSseHandlers(
  sseState: SseHandlerDeps,
  chatState: ChatStateResult & { setSessions: (s: import("../types").ChatSession[]) => void },
) {
  const { latestEvent, loadHistory } = sseState;
  const {
    viewSessionId,
    runningSessionId,
    setRunningSessionId,
    setIsStreaming,
    setSessionPendingQuestion,
    setSessions,
    addMessage,
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

    switch (event.type) {
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
        // Remove the thinking indicator if this is the first text
        if (thinkingMsgId.current) {
          // Seal the thinking message (keep it, just mark not active)
          thinkingMsgId.current = null;
        }
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
        thinkingMsgId.current = null;
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
        const { request_id, questions } = event.payload;
        thinkingMsgId.current = null;
        assistantMsgId.current = null;
        setSessionPendingQuestion(session, { requestId: request_id, questions });
        break;
      }

      case "done": {
        const { session_id } = event.payload;
        const completedSession = runningRef.current;
        setRunningSessionId(null);
        setIsStreaming(false);
        setSessionPendingQuestion(completedSession, null);
        thinkingMsgId.current = null;
        assistantMsgId.current = null;
        toolIdToMsgId.current.clear();

        if (session_id && completedSession === viewRef.current) {
          const msgs = getMessages(completedSession);
          setMessages(session_id, msgs);
          setViewSessionId(session_id);
        }

        loadHistory().then(setSessions).catch(console.error);
        break;
      }

      case "error_event": {
        const { message } = event.payload;
        setRunningSessionId(null);
        setIsStreaming(false);
        setSessionPendingQuestion(session, null);
        thinkingMsgId.current = null;
        assistantMsgId.current = null;
        toolIdToMsgId.current.clear();
        addMessage(session, {
          id: generateId(),
          type: "error",
          content: message,
          timestamp: Date.now(),
        });
        break;
      }
    }
  }, [latestEvent]); // eslint-disable-line react-hooks/exhaustive-deps
}

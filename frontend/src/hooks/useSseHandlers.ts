import { useEffect, useRef, type MutableRefObject } from "react";
import type { ChatMessage, ChatSession, SseEvent, TranscriptMessage } from "../types";
import type { ChatStateResult } from "./useChatState";
import { buildMessagesFromTranscript } from "../utils/transcript";

interface SseHandlerDeps {
  eventQueueRef: MutableRefObject<SseEvent[]>;
  eventSeq: number;
  loadHistory: () => Promise<import("../types").ChatSession[]>;
  loadTranscript: (sessionId: string, projectDir: string, signal?: AbortSignal) => Promise<TranscriptMessage[]>;
  sessions: ChatSession[];
  vmId: string;
}

export function useSseHandlers(
  sseState: SseHandlerDeps,
  chatState: ChatStateResult & { setSessions: (s: import("../types").ChatSession[]) => void },
) {
  const { eventQueueRef, eventSeq, loadHistory, loadTranscript, sessions, vmId } = sseState;
  const {
    viewSessionId,
    runningSessionId,
    setRunningSessionId,
    setIsStreaming,
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

  // Track the current task_id for message persistence
  const currentTaskIdRef = useRef<string | null>(null);

  // Track pending tool message IDs by tool_use_id
  const toolIdToMsgId = useRef<Map<string, string>>(new Map());

  // Track current thinking message id
  const thinkingMsgId = useRef<string | null>(null);

  // Track current assistant message id (accumulating text)
  const assistantMsgId = useRef<string | null>(null);

  useEffect(() => {
    // Drain all queued events in one effect run. React 18 auto-batches the
    // setState(seq+1) calls from SSE listeners so multiple rapid events are
    // processed together here without flushSync.
    const events = eventQueueRef.current.splice(0);
    for (const event of events) {
      handleEvent(event);
    }

    function handleEvent(event: SseEvent) {
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
        case "relay_ready":
          break;

        case "session_start": {
          const { task_id } = event.payload;
          currentTaskIdRef.current = task_id;
          setTaskId(session, task_id);
          const project_dir = sessions.find((s) => s.session_id === session)?.project_dir ?? null;
          localStorage.setItem(
            `chat_running_task_${vmId}`,
            JSON.stringify({ task_id, running_session_id: session, project_dir }),
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
          if (currentTaskIdRef.current) {
            localStorage.setItem(`chat_messages_task_${currentTaskIdRef.current}`, JSON.stringify(getMessages(session)));
          }
          break;
        }

        case "thinking_delta": {
          const { thinking } = event.payload;
          if (thinkingMsgId.current) {
            updateMessageById(session, thinkingMsgId.current, (m) => ({
              ...m,
              content: m.content + thinking,
            }));
            if (currentTaskIdRef.current) {
              localStorage.setItem(`chat_messages_task_${currentTaskIdRef.current}`, JSON.stringify(getMessages(session)));
            }
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
          if (currentTaskIdRef.current) {
            localStorage.setItem(`chat_messages_task_${currentTaskIdRef.current}`, JSON.stringify(getMessages(session)));
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
          if (currentTaskIdRef.current) {
            localStorage.setItem(`chat_messages_task_${currentTaskIdRef.current}`, JSON.stringify(getMessages(session)));
          }
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
            if (currentTaskIdRef.current) {
              localStorage.setItem(`chat_messages_task_${currentTaskIdRef.current}`, JSON.stringify(getMessages(session)));
            }
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
          const { session_id, task_id } = event.payload;
          const completedSession = runningRef.current;
          localStorage.removeItem(`chat_messages_task_${task_id}`);
          currentTaskIdRef.current = null;
          setRunningSessionId(null);
          setIsStreaming(false);
          setSessionPendingQuestion(completedSession, null);
          sealThinking();
          assistantMsgId.current = null;
          toolIdToMsgId.current.clear();

          if (session_id) {
            if (completedSession !== session_id) {
              const msgs = getMessages(completedSession);
              setMessages(session_id, msgs);
              setMessages(completedSession, []);
            }
            if (completedSession === viewRef.current) {
              setViewSessionId(session_id);
            }
          }

          loadHistory().then(setSessions).catch(console.error);
          break;
        }

        case "error_event": {
          const { message } = event.payload;
          if (currentTaskIdRef.current) {
            localStorage.removeItem(`chat_messages_task_${currentTaskIdRef.current}`);
            currentTaskIdRef.current = null;
          }
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

        case "reconnecting": {
          const { task_id, running_session_id: rawRunningId, project_dir } = event.payload;
          // onopen fires for every EventSource connection; deduplicate by task_id.
          if (currentTaskIdRef.current === task_id) break;
          // Legacy localStorage may have stored null for new-chat sessions; generate a UUID for those.
          const running_session_id = rawRunningId ?? crypto.randomUUID();
          currentTaskIdRef.current = task_id;
          setTaskId(running_session_id, task_id);
          setRunningSessionId(running_session_id);
          setIsStreaming(true);
          setViewSessionId(running_session_id);
          if (running_session_id && !sessions.find((s) => s.session_id === running_session_id)) {
            setSessions([
              { session_id: running_session_id, created_at: new Date().toISOString(), title: "New chat\u2026", is_pending: true },
              ...sessions,
            ]);
          }
          let inProgressMessages: ChatMessage[] = [];
          const savedMessages = localStorage.getItem(`chat_messages_task_${task_id}`);
          if (savedMessages) {
            try {
              inProgressMessages = JSON.parse(savedMessages) as ChatMessage[];
              setMessages(running_session_id, inProgressMessages);
            } catch {
              // ignore parse errors
            }
          }
          if (running_session_id && project_dir) {
            loadTranscript(running_session_id, project_dir).then((transcript) => {
              const historical = buildMessagesFromTranscript(transcript);
              if (historical.length > 0) {
                setMessages(running_session_id, [...historical, ...inProgressMessages]);
              }
            }).catch(console.error);
          }
          break;
        }
      }
    }
  }, [eventSeq]); // eslint-disable-line react-hooks/exhaustive-deps
}

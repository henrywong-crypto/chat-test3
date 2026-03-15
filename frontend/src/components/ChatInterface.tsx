import React, { useCallback, useEffect, useRef, useState } from "react";
import type { ChatMessage, ChatSession, ContentBlock, TranscriptMessage } from "../types";
import { useSse } from "../contexts/SseContext";
import { useChatState } from "../hooks/useChatState";
import { useSseHandlers } from "../hooks/useSseHandlers";
import AskUserQuestionPanel from "./AskUserQuestionPanel";
import ChatComposer from "./ChatComposer";
import ChatMessagesPane from "./ChatMessagesPane";
import ClaudeStatus from "./ClaudeStatus";

function extractToolResultContent(raw: string | ContentBlock[] | undefined): string {
  if (!raw) return "";
  if (typeof raw === "string") return raw;
  return raw.map((b) => b.text ?? "").join("");
}

function buildMessagesFromTranscript(transcript: TranscriptMessage[]): ChatMessage[] {
  const messages: ChatMessage[] = [];
  let id = 0;
  const nextId = () => `t${id++}`;

  // toolId → index in messages array, for attaching tool results
  const toolIdToIndex = new Map<string, number>();

  for (const entry of transcript) {
    const role = entry.role;

    if (role === "user") {
      const blocks = typeof entry.content === "string" ? null : entry.content;
      if (blocks) {
        // Attach tool results to their corresponding tool use messages
        for (const block of blocks) {
          if (block.type === "tool_result" && block.tool_use_id) {
            const idx = toolIdToIndex.get(block.tool_use_id);
            if (idx !== undefined) {
              messages[idx] = {
                ...messages[idx],
                toolResult: {
                  content: extractToolResultContent(block.content),
                  isError: block.is_error ?? false,
                },
              };
            }
          }
        }
        // Also collect any plain text from user turns
        const text = blocks.map((b) => (b.type === "text" ? b.text ?? "" : "")).join("");
        if (text.trim()) {
          messages.push({ id: nextId(), type: "user", content: text, timestamp: Date.now() });
        }
      } else if (typeof entry.content === "string" && entry.content.trim()) {
        messages.push({ id: nextId(), type: "user", content: entry.content, timestamp: Date.now() });
      }
    } else if (role === "assistant") {
      const blocks = typeof entry.content === "string"
        ? [{ type: "text", text: entry.content }]
        : entry.content;

      for (const block of blocks) {
        if (block.type === "thinking" && block.thinking) {
          messages.push({
            id: nextId(),
            type: "assistant",
            content: block.thinking,
            timestamp: Date.now(),
            isThinking: true,
          });
        } else if (block.type === "text" && block.text) {
          messages.push({
            id: nextId(),
            type: "assistant",
            content: block.text,
            timestamp: Date.now(),
          });
        } else if (block.type === "tool_use") {
          const msgIdx = messages.length;
          if (block.id) toolIdToIndex.set(block.id, msgIdx);
          messages.push({
            id: nextId(),
            type: "tool",
            content: "",
            timestamp: Date.now(),
            isToolUse: true,
            toolId: block.id,
            toolName: block.name,
            toolInput: block.input as Record<string, unknown>,
          });
        }
      }
    }
  }
  return messages;
}

interface ChatInterfaceProps {
  sessions: ChatSession[];
  setSessions: (s: ChatSession[]) => void;
  selectedSession: ChatSession | null;
  newChatKey: number;
  onRunningSessionChange?: (sessionId: string | null) => void;
}

export default function ChatInterface({ sessions, setSessions, selectedSession, newChatKey, onRunningSessionChange }: ChatInterfaceProps) {
  const sseCtx = useSse();
  const { loadHistory, loadTranscript } = sseCtx;
  const chatState = useChatState();
  const newChatKeyRef = useRef(newChatKey);
  newChatKeyRef.current = newChatKey;
  // Snapshot of newChatKey at the time the current null-session request was sent
  const sessionStartKeyRef = useRef(newChatKey);

  // Bumped whenever the composer should receive focus (new chat or session switch)
  const [composerFocusKey, setComposerFocusKey] = useState(0);

  const {
    viewSessionId,
    setViewSessionId,
    runningSessionId,
    setRunningSessionId,
    isStreaming,
    setIsStreaming,
    nullOrphaned,
    setNullOrphaned,
    getSessionPendingQuestion,
    setSessionPendingQuestion,
    getTaskId,
    getMessages,
    addMessage,
    setMessages,
    generateId,
  } = chatState;

  // Wire SSE events to chat state
  useSseHandlers(
    { latestEvent: sseCtx.latestEvent, loadHistory, newChatKeyRef, sessionStartKeyRef, vmId: sseCtx.vmId },
    { ...chatState, setSessions },
  );

  // Load history on mount
  useEffect(() => {
    loadHistory().then(setSessions).catch(console.error);
  }, [loadHistory, setSessions]);

  // Load transcript when user switches to an existing session
  const loadTranscriptForSession = useCallback(async (session: ChatSession) => {
    if (!session.project_dir) return;
    if (getMessages(session.session_id).length > 0) return;
    try {
      const transcript = await loadTranscript(session.session_id, session.project_dir);
      const msgs = buildMessagesFromTranscript(transcript);
      setMessages(session.session_id, msgs);
    } catch (err) {
      console.error("Failed to load transcript", err);
    }
  }, [loadTranscript, getMessages, setMessages]);

  // React to session selection from the sidebar (driven by App.tsx)
  useEffect(() => {
    if (!selectedSession) {
      setViewSessionId(null);
      return;
    }
    setViewSessionId(selectedSession.session_id);
    loadTranscriptForSession(selectedSession);
    setComposerFocusKey((k) => k + 1);
  }, [selectedSession, setViewSessionId, loadTranscriptForSession]);

  // Refs for reading latest values inside effects without adding to deps
  const runningSessionIdRef = useRef(runningSessionId);
  runningSessionIdRef.current = runningSessionId;
  const isStreamingRef = useRef(isStreaming);
  isStreamingRef.current = isStreaming;

  // Reset to a blank new chat when the user explicitly clicks "New Chat"
  useEffect(() => {
    if (newChatKey === 0) return;
    if (runningSessionIdRef.current === null && isStreamingRef.current) {
      setNullOrphaned(true);
    }
    setMessages(null, []);
    setViewSessionId(null);
    setComposerFocusKey((k) => k + 1);
  }, [newChatKey, setMessages, setViewSessionId, setNullOrphaned]);

  // Notify parent when the running session changes so Sidebar can show the active indicator
  const onRunningSessionChangeRef = useRef(onRunningSessionChange);
  onRunningSessionChangeRef.current = onRunningSessionChange;
  useEffect(() => {
    onRunningSessionChangeRef.current?.(runningSessionId);
  }, [runningSessionId]);

  const handleSend = useCallback(async (text: string) => {
    const sessionId = viewSessionId;
    if (sessionId === null) {
      sessionStartKeyRef.current = newChatKey;
    }
    const userMsgId = generateId();
    addMessage(sessionId, {
      id: userMsgId,
      type: "user",
      content: text,
      timestamp: Date.now(),
    });
    setRunningSessionId(sessionId);
    setIsStreaming(true);

    try {
      await sseCtx.sendQuery(text, sessionId);
    } catch (err) {
      addMessage(sessionId, {
        id: generateId(),
        type: "error",
        content: String(err),
        timestamp: Date.now(),
      });
      setRunningSessionId(null);
      setIsStreaming(false);
    }
  }, [viewSessionId, generateId, addMessage, setRunningSessionId, setIsStreaming, sseCtx]);

  const handleStop = useCallback(() => {
    sseCtx.sendStop(getTaskId(runningSessionId) ?? "").catch(console.error);
  }, [sseCtx, getTaskId, runningSessionId]);

  const handleAnswerQuestion = useCallback(
    async (requestId: string, answers: Record<string, string>) => {
      const taskId = getSessionPendingQuestion(viewSessionId)?.taskId ?? "";
      setSessionPendingQuestion(viewSessionId, null);
      await sseCtx.answerQuestion(taskId, requestId, answers);
    },
    [sseCtx, setSessionPendingQuestion, getSessionPendingQuestion, viewSessionId],
  );

  const handleSkipQuestion = useCallback(
    async (requestId: string) => {
      const taskId = getSessionPendingQuestion(viewSessionId)?.taskId ?? "";
      setSessionPendingQuestion(viewSessionId, null);
      await sseCtx.answerQuestion(taskId, requestId, {});
    },
    [sseCtx, setSessionPendingQuestion, getSessionPendingQuestion, viewSessionId],
  );

  const messages = getMessages(viewSessionId);
  const pendingQuestion = getSessionPendingQuestion(viewSessionId);
  const isCurrentRunning = isStreaming && runningSessionId === viewSessionId && !nullOrphaned;
  const isOtherRunning = isStreaming && runningSessionId !== viewSessionId && !nullOrphaned;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChatMessagesPane messages={messages} isLoading={isCurrentRunning} />
      <ClaudeStatus isLoading={isCurrentRunning} onAbort={handleStop} />
      {pendingQuestion ? (
        <div className="flex-shrink-0 border-t border-border p-4">
          <div className="mx-auto max-w-3xl">
            <AskUserQuestionPanel
              pendingQuestion={pendingQuestion}
              onSubmit={handleAnswerQuestion}
              onSkip={handleSkipQuestion}
            />
          </div>
        </div>
      ) : (
        <ChatComposer
          isLoading={isCurrentRunning}
          isOtherRunning={isOtherRunning}
          onSend={handleSend}
          onStop={handleStop}
          focusKey={composerFocusKey}
        />
      )}
    </div>
  );
}

import React, { useCallback, useEffect, useRef } from "react";
import type { ChatMessage, ChatSession, TranscriptMessage } from "../types";
import { useSse } from "../contexts/SseContext";
import { useChatState } from "../hooks/useChatState";
import { useSseHandlers } from "../hooks/useSseHandlers";
import AskUserQuestionPanel from "./AskUserQuestionPanel";
import ChatComposer from "./ChatComposer";
import ChatMessagesPane from "./ChatMessagesPane";
import ClaudeStatus from "./ClaudeStatus";

function buildMessagesFromTranscript(transcript: TranscriptMessage[]): ChatMessage[] {
  const messages: ChatMessage[] = [];
  let id = 0;
  const nextId = () => `t${id++}`;

  for (const entry of transcript) {
    const role = entry.role;

    if (role === "user") {
      const content = typeof entry.content === "string"
        ? entry.content
        : entry.content.map((b) => (b.type === "text" ? b.text ?? "" : "")).join("");
      if (content.trim()) {
        messages.push({ id: nextId(), type: "user", content, timestamp: Date.now() });
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
  onRunningSessionChange?: (sessionId: string | null) => void;
}

export default function ChatInterface({ sessions, setSessions, selectedSession, onRunningSessionChange }: ChatInterfaceProps) {
  const sseCtx = useSse();
  const chatState = useChatState();

  const {
    viewSessionId,
    setViewSessionId,
    runningSessionId,
    setRunningSessionId,
    isStreaming,
    setIsStreaming,
    getSessionPendingQuestion,
    setSessionPendingQuestion,
    getMessages,
    addMessage,
    setMessages,
    generateId,
  } = chatState;

  // Wire SSE events to chat state
  useSseHandlers(
    { latestEvent: sseCtx.latestEvent, loadHistory: sseCtx.loadHistory },
    { ...chatState, setSessions },
  );

  // Load history on mount
  useEffect(() => {
    sseCtx.loadHistory().then(setSessions).catch(console.error);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Load transcript when user switches to an existing session
  const loadTranscriptForSession = useCallback(async (session: ChatSession) => {
    if (!session.project_dir) return;
    if (getMessages(session.session_id).length > 0) return;
    try {
      const transcript = await sseCtx.loadTranscript(session.session_id, session.project_dir);
      const msgs = buildMessagesFromTranscript(transcript);
      setMessages(session.session_id, msgs);
    } catch (err) {
      console.error("Failed to load transcript", err);
    }
  }, [sseCtx, getMessages, setMessages]);

  // React to session selection from the sidebar (driven by App.tsx)
  useEffect(() => {
    if (!selectedSession) {
      setViewSessionId(null);
      return;
    }
    setViewSessionId(selectedSession.session_id);
    loadTranscriptForSession(selectedSession);
  }, [selectedSession]); // eslint-disable-line react-hooks/exhaustive-deps

  // Notify parent when the running session changes so Sidebar can show the active indicator
  const onRunningSessionChangeRef = useRef(onRunningSessionChange);
  onRunningSessionChangeRef.current = onRunningSessionChange;
  useEffect(() => {
    onRunningSessionChangeRef.current?.(runningSessionId);
  }, [runningSessionId]);

  const handleSend = useCallback(async (text: string) => {
    const sessionId = viewSessionId;
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
    sseCtx.sendStop().catch(console.error);
  }, [sseCtx]);

  const handleAnswerQuestion = useCallback(
    async (requestId: string, answers: Record<string, string>) => {
      setSessionPendingQuestion(viewSessionId, null);
      await sseCtx.answerQuestion(requestId, answers);
    },
    [sseCtx, setSessionPendingQuestion, viewSessionId],
  );

  const handleSkipQuestion = useCallback(
    async (requestId: string) => {
      setSessionPendingQuestion(viewSessionId, null);
      await sseCtx.answerQuestion(requestId, {});
    },
    [sseCtx, setSessionPendingQuestion, viewSessionId],
  );

  const messages = getMessages(viewSessionId);
  const pendingQuestion = getSessionPendingQuestion(viewSessionId);
  const isLoading = isStreaming && runningSessionId === viewSessionId;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChatMessagesPane messages={messages} isLoading={isLoading} />
      <ClaudeStatus isLoading={isLoading} onAbort={handleStop} />
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
          isLoading={isLoading}
          onSend={handleSend}
          onStop={handleStop}
        />
      )}
    </div>
  );
}

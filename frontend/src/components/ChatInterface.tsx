import React, { useCallback, useEffect, useRef, useState } from "react";
import type { ChatMessage, ChatSession } from "../types";
import { useSse } from "../contexts/SseContext";
import { useChatState } from "../hooks/useChatState";
import { useSseHandlers } from "../hooks/useSseHandlers";
import { buildMessagesFromTranscript } from "../utils/transcript";
import AskUserQuestionPanel from "./AskUserQuestionPanel";
import ChatComposer from "./ChatComposer";
import ChatMessagesPane from "./ChatMessagesPane";
import ClaudeStatus from "./ClaudeStatus";

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
  // Snapshot of newChatKey at the time the current pending-session request was sent
  const sessionStartKeyRef = useRef(newChatKey);

  // Each new (pre-session) chat gets a unique "pending:UUID" key instead of sharing null,
  // so concurrent new chats don't clobber each other's messages.
  const pendingSessionIdRef = useRef(`pending:${Math.random().toString(36).slice(2, 10)}`);

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
    { eventQueueRef: sseCtx.eventQueueRef, eventSeq: sseCtx.eventSeq, loadHistory, loadTranscript, newChatKeyRef, sessionStartKeyRef, sessions, vmId: sseCtx.vmId },
    { ...chatState, setSessions },
  );

  // Load history on mount and set initial viewSessionId to the pending slot
  useEffect(() => {
    setViewSessionId(pendingSessionIdRef.current);
    loadHistory().then(setSessions).catch(console.error);
  }, [loadHistory, setSessions, setViewSessionId]); // eslint-disable-line react-hooks/exhaustive-deps

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
      setViewSessionId(pendingSessionIdRef.current);
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
    if (runningSessionIdRef.current?.startsWith("pending:") && isStreamingRef.current) {
      setNullOrphaned(true);
    }
    // Clear the current pending slot and rotate to a fresh key so this chat's
    // messages don't collide with the previous (or any future) new chat.
    setMessages(pendingSessionIdRef.current, []);
    const freshPendingId = `pending:${Math.random().toString(36).slice(2, 10)}`;
    pendingSessionIdRef.current = freshPendingId;
    setViewSessionId(freshPendingId);
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
    const isPending = sessionId?.startsWith("pending:") ?? true;
    if (isPending) {
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

    // Pending sessions are new chats — the server expects null for session_id
    const serverSessionId = isPending ? null : sessionId;
    try {
      await sseCtx.sendQuery(text, serverSessionId);
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
  }, [viewSessionId, newChatKey, generateId, addMessage, setRunningSessionId, setIsStreaming, sseCtx]);

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
          isVmReady={sseCtx.isVmReady}
          onSend={handleSend}
          onStop={handleStop}
          focusKey={composerFocusKey}
        />
      )}
    </div>
  );
}

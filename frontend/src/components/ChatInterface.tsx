import React, { useCallback, useEffect, useRef, useState } from "react";
import type { ChatSession } from "../types";
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
  onRunningSessionChange?: (sessionId: string | null) => void;
}

export default function ChatInterface({ sessions, setSessions, selectedSession, onRunningSessionChange }: ChatInterfaceProps) {
  const sseCtx = useSse();
  const { loadHistory, loadTranscript } = sseCtx;
  const chatState = useChatState();

  // Bumped whenever the composer should receive focus (new chat or session switch)
  const [composerFocusKey, setComposerFocusKey] = useState(0);

  const {
    viewSessionId,
    setViewSessionId,
    runningSessionId,
    setRunningSessionId,
    isStreaming,
    setIsStreaming,
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
    { eventQueueRef: sseCtx.eventQueueRef, eventSeq: sseCtx.eventSeq, loadHistory, loadTranscript, sessions, vmId: sseCtx.vmId },
    { ...chatState, setSessions },
  );

  // Load history on mount
  useEffect(() => {
    loadHistory().then(setSessions).catch(console.error);
  }, [loadHistory, setSessions]); // eslint-disable-line react-hooks/exhaustive-deps

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
      setComposerFocusKey((k) => k + 1);
      return;
    }
    setViewSessionId(selectedSession.session_id);
    if (!selectedSession.is_pending) {
      loadTranscriptForSession(selectedSession);
    }
    setComposerFocusKey((k) => k + 1);
  }, [selectedSession, setViewSessionId, loadTranscriptForSession]);

  // Notify parent when the running session changes so Sidebar can show the active indicator
  const onRunningSessionChangeRef = useRef(onRunningSessionChange);
  onRunningSessionChangeRef.current = onRunningSessionChange;
  useEffect(() => {
    onRunningSessionChangeRef.current?.(runningSessionId);
  }, [runningSessionId]);

  const selectedSessionRef = useRef(selectedSession);
  selectedSessionRef.current = selectedSession;

  const handleSend = useCallback(async (text: string) => {
    const conversationId = viewSessionId;
    const effectiveId = conversationId ?? crypto.randomUUID();
    const isNewConversation = !conversationId || (selectedSessionRef.current?.is_pending ?? false);
    const serverSessionId = isNewConversation ? null : effectiveId;

    addMessage(effectiveId, {
      id: generateId(),
      type: "user",
      content: text,
      timestamp: Date.now(),
    });
    setRunningSessionId(effectiveId);
    setIsStreaming(true);

    if (!conversationId) {
      setViewSessionId(effectiveId);
      setSessions((prev: ChatSession[]) => [
        { session_id: effectiveId, created_at: new Date().toISOString(), title: "New chat\u2026", is_pending: true },
        ...prev,
      ]);
    } else if (isNewConversation && selectedSessionRef.current) {
      setSessions((prev: ChatSession[]) => [selectedSessionRef.current!, ...prev]);
    }

    try {
      await sseCtx.sendQuery(text, serverSessionId);
    } catch (err) {
      if (isNewConversation) {
        setSessions((prev: ChatSession[]) => prev.filter((s) => s.session_id !== effectiveId));
      }
      addMessage(effectiveId, {
        id: generateId(),
        type: "error",
        content: String(err),
        timestamp: Date.now(),
      });
      setRunningSessionId(null);
      setIsStreaming(false);
    }
  }, [viewSessionId, generateId, addMessage, setRunningSessionId, setIsStreaming, setViewSessionId, setSessions, sseCtx]);

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
  const isCurrentRunning = isStreaming && runningSessionId === viewSessionId;
  const isOtherRunning = isStreaming && runningSessionId !== viewSessionId;

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

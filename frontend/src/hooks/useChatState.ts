import { useCallback, useRef, useState } from "react";
import type { ChatMessage, ChatSession, PendingQuestion } from "../types";

function generateId(): string {
  return Math.random().toString(36).slice(2, 10);
}

export interface ChatStateResult {
  messagesBySession: React.MutableRefObject<Map<string | null, ChatMessage[]>>;
  viewSessionId: string | null;
  setViewSessionId: (id: string | null) => void;
  runningSessionId: string | null;
  setRunningSessionId: (id: string | null) => void;
  isStreaming: boolean;
  setIsStreaming: (v: boolean) => void;
  getSessionPendingQuestion: (sessionId: string | null) => PendingQuestion | null;
  setSessionPendingQuestion: (sessionId: string | null, q: PendingQuestion | null) => void;
  sessions: ChatSession[];
  setSessions: (s: ChatSession[]) => void;
  getMessages: (sessionId: string | null) => ChatMessage[];
  setMessages: (sessionId: string | null, msgs: ChatMessage[]) => void;
  addMessage: (sessionId: string | null, msg: ChatMessage) => void;
  updateLastMessage: (sessionId: string | null, updater: (msg: ChatMessage) => ChatMessage) => void;
  updateMessageById: (sessionId: string | null, id: string, updater: (msg: ChatMessage) => ChatMessage) => void;
  generateId: () => string;
  renderTick: number;
  bumpRender: () => void;
}

// React is imported for the MutableRefObject type annotation
import React from "react";

export function useChatState(): ChatStateResult {
  const messagesBySession = useRef<Map<string | null, ChatMessage[]>>(new Map());
  const pendingQuestionsBySession = useRef<Map<string | null, PendingQuestion>>(new Map());
  const [viewSessionId, setViewSessionId] = useState<string | null>(null);
  const [runningSessionId, setRunningSessionId] = useState<string | null>(null);
  const [isStreaming, setIsStreaming] = useState(false);
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [renderTick, setRenderTick] = useState(0);

  const bumpRender = useCallback(() => setRenderTick((t) => t + 1), []);

  const getSessionPendingQuestion = useCallback((sessionId: string | null): PendingQuestion | null => {
    return pendingQuestionsBySession.current.get(sessionId) ?? null;
  }, []);

  const setSessionPendingQuestion = useCallback((sessionId: string | null, q: PendingQuestion | null) => {
    if (q === null) {
      pendingQuestionsBySession.current.delete(sessionId);
    } else {
      pendingQuestionsBySession.current.set(sessionId, q);
    }
    setRenderTick((t) => t + 1);
  }, []);

  const getMessages = useCallback((sessionId: string | null): ChatMessage[] => {
    return messagesBySession.current.get(sessionId) ?? [];
  }, []);

  const setMessages = useCallback((sessionId: string | null, msgs: ChatMessage[]) => {
    messagesBySession.current.set(sessionId, msgs);
    setRenderTick((t) => t + 1);
  }, []);

  const addMessage = useCallback((sessionId: string | null, msg: ChatMessage) => {
    const prev = messagesBySession.current.get(sessionId) ?? [];
    messagesBySession.current.set(sessionId, [...prev, msg]);
    setRenderTick((t) => t + 1);
  }, []);

  const updateLastMessage = useCallback((
    sessionId: string | null,
    updater: (msg: ChatMessage) => ChatMessage,
  ) => {
    const msgs = messagesBySession.current.get(sessionId) ?? [];
    if (msgs.length === 0) return;
    const updated = [...msgs];
    updated[updated.length - 1] = updater(updated[updated.length - 1]);
    messagesBySession.current.set(sessionId, updated);
    setRenderTick((t) => t + 1);
  }, []);

  const updateMessageById = useCallback((
    sessionId: string | null,
    id: string,
    updater: (msg: ChatMessage) => ChatMessage,
  ) => {
    const msgs = messagesBySession.current.get(sessionId) ?? [];
    const updated = msgs.map((m) => (m.id === id ? updater(m) : m));
    messagesBySession.current.set(sessionId, updated);
    setRenderTick((t) => t + 1);
  }, []);

  return {
    messagesBySession,
    viewSessionId,
    setViewSessionId,
    runningSessionId,
    setRunningSessionId,
    isStreaming,
    setIsStreaming,
    getSessionPendingQuestion,
    setSessionPendingQuestion,
    sessions,
    setSessions,
    getMessages,
    setMessages,
    addMessage,
    updateLastMessage,
    updateMessageById,
    generateId,
    renderTick,
    bumpRender,
  };
}

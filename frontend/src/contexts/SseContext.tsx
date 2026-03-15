import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type {
  ChatSession,
  Conversation,
  FileEntry,
  SseAskUserQuestion,
  SseDone,
  SseErrorEvent,
  SseEvent,
  SseSessionStart,
  SseTaskCreated,
  SseTextDelta,
  SseThinkingDelta,
  SseToolResult,
  SseToolStart,
  StoredQuestion,
  TranscriptMessage,
} from "../types";

interface SseContextValue {
  vmId: string;
  csrfToken: string;
  uploadDir: string;
  uploadAction: string;
  hasUserRootfs: boolean;
  eventQueueRef: React.MutableRefObject<SseEvent[]>;
  eventSeq: number;
  conversations: Conversation[];
  createConversation: () => Conversation;
  updateConversation: (id: string, update: Partial<Conversation>) => void;
  deleteConversation: (id: string) => void;
  syncConversationsFromHistory: () => Promise<void>;
  sendQuery: (content: string, conversationId: string, sessionId?: string, workDir?: string) => void;
  sendStop: (taskId: string) => Promise<void>;
  answerQuestion: (taskId: string, requestId: string, answers: Record<string, string>) => Promise<void>;
  loadHistory: () => Promise<ChatSession[]>;
  loadTranscript: (sessionId: string, projectDir: string, signal?: AbortSignal) => Promise<TranscriptMessage[]>;
  deleteSession: (sessionId: string, projectDir: string) => Promise<void>;
  listFiles: (path: string) => Promise<FileEntry[]>;
  storeQuestion: (requestId: string, data: StoredQuestion) => void;
  clearQuestion: (requestId: string) => void;
  getQuestionsForConversation: (conversationId: string) => StoredQuestion | null;
}

const SseContext = createContext<SseContextValue | null>(null);

function readAppConfig(): {
  vmId: string;
  csrfToken: string;
  uploadDir: string;
  uploadAction: string;
  hasUserRootfs: boolean;
} {
  const el = document.getElementById("app-config");
  return {
    vmId: el?.dataset.vmId ?? "",
    csrfToken: el?.dataset.csrfToken ?? "",
    uploadDir: el?.dataset.uploadDir ?? "/tmp",
    uploadAction: el?.dataset.uploadAction ?? "",
    hasUserRootfs: el?.dataset.hasUserRootfs === "true",
  };
}

function loadConversationsFromStorage(vmId: string): Conversation[] {
  try {
    const saved = localStorage.getItem(`conversations_${vmId}`);
    return saved ? (JSON.parse(saved) as Conversation[]) : [];
  } catch {
    return [];
  }
}

function saveConversationsToStorage(vmId: string, conversations: Conversation[]): void {
  localStorage.setItem(`conversations_${vmId}`, JSON.stringify(conversations));
}

function parseSseBlock(part: string): { eventName: string; data: string } | null {
  let eventName = "";
  let data = "";
  for (const line of part.split("\n")) {
    if (line.startsWith("event: ")) {
      eventName = line.slice(7).trim();
    } else if (line.startsWith("data: ")) {
      data = line.slice(6);
    }
  }
  return eventName && data ? { eventName, data } : null;
}

function dispatchSseEvent(
  eventName: string,
  data: string,
  pushEvent: (e: SseEvent) => void,
  vmId: string,
): void {
  let payload: unknown;
  try {
    payload = JSON.parse(data);
  } catch {
    return;
  }
  switch (eventName) {
    case "task_created":
      pushEvent({ type: "task_created", payload: payload as SseTaskCreated });
      break;
    case "session_start":
      pushEvent({ type: "session_start", payload: payload as SseSessionStart });
      break;
    case "init":
      pushEvent({ type: "init" });
      break;
    case "text_delta":
      pushEvent({ type: "text_delta", payload: payload as SseTextDelta });
      break;
    case "thinking_delta":
      pushEvent({ type: "thinking_delta", payload: payload as SseThinkingDelta });
      break;
    case "tool_start":
      pushEvent({ type: "tool_start", payload: payload as SseToolStart });
      break;
    case "ask_user_question":
      pushEvent({ type: "ask_user_question", payload: payload as SseAskUserQuestion });
      break;
    case "tool_result":
      pushEvent({ type: "tool_result", payload: payload as SseToolResult });
      break;
    case "done":
      localStorage.removeItem(`chat_running_task_${vmId}`);
      pushEvent({ type: "done", payload: payload as SseDone });
      break;
    case "error_event":
      localStorage.removeItem(`chat_running_task_${vmId}`);
      pushEvent({ type: "error_event", payload: payload as SseErrorEvent });
      break;
  }
}

async function readFetchSseStream(
  response: Response,
  pushEvent: (e: SseEvent) => void,
  vmId: string,
): Promise<void> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const parts = buffer.split("\n\n");
      buffer = parts.pop() ?? "";
      for (const part of parts) {
        if (!part.trim()) continue;
        const block = parseSseBlock(part);
        if (block) {
          dispatchSseEvent(block.eventName, block.data, pushEvent, vmId);
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function attachEventSourceListeners(
  es: EventSource,
  pushEvent: (e: SseEvent) => void,
  vmId: string,
): void {
  const add = (name: string, handler: (e: MessageEvent) => void) => {
    es.addEventListener(name, handler as EventListener);
  };
  add("task_created", (e) => pushEvent({ type: "task_created", payload: JSON.parse(e.data) as SseTaskCreated }));
  add("session_start", (e) => pushEvent({ type: "session_start", payload: JSON.parse(e.data) as SseSessionStart }));
  add("init", () => pushEvent({ type: "init" }));
  add("text_delta", (e) => pushEvent({ type: "text_delta", payload: JSON.parse(e.data) as SseTextDelta }));
  add("thinking_delta", (e) => pushEvent({ type: "thinking_delta", payload: JSON.parse(e.data) as SseThinkingDelta }));
  add("tool_start", (e) => pushEvent({ type: "tool_start", payload: JSON.parse(e.data) as SseToolStart }));
  add("ask_user_question", (e) => pushEvent({ type: "ask_user_question", payload: JSON.parse(e.data) as SseAskUserQuestion }));
  add("tool_result", (e) => pushEvent({ type: "tool_result", payload: JSON.parse(e.data) as SseToolResult }));
  add("done", (e) => {
    localStorage.removeItem(`chat_running_task_${vmId}`);
    pushEvent({ type: "done", payload: JSON.parse(e.data) as SseDone });
  });
  add("error_event", (e) => {
    localStorage.removeItem(`chat_running_task_${vmId}`);
    pushEvent({ type: "error_event", payload: JSON.parse(e.data) as SseErrorEvent });
  });
  es.onerror = () => {
    es.close();
  };
}

export function SseProvider({ children }: { children: React.ReactNode }) {
  const config = useRef(readAppConfig());
  const { vmId, uploadDir, uploadAction, hasUserRootfs } = config.current;

  const csrfTokenRef = useRef(config.current.csrfToken);
  const [csrfToken, setCsrfToken] = useState(config.current.csrfToken);

  const refreshCsrfToken = useCallback((res: Response) => {
    const newToken = res.headers.get("x-csrf-token");
    if (newToken) {
      csrfTokenRef.current = newToken;
      setCsrfToken(newToken);
    }
  }, []);

  const eventQueueRef = useRef<SseEvent[]>([]);
  const [eventSeq, setEventSeq] = useState(0);

  const pushEvent = useCallback((event: SseEvent) => {
    eventQueueRef.current.push(event);
    setEventSeq((s) => s + 1);
  }, []);

  const [conversations, setConversations] = useState<Conversation[]>(() =>
    loadConversationsFromStorage(vmId)
  );

  const createConversation = useCallback((): Conversation => {
    const conversation: Conversation = {
      conversationId: crypto.randomUUID(),
      createdAt: Date.now(),
    };
    setConversations((prev) => {
      const updated = [conversation, ...prev];
      saveConversationsToStorage(vmId, updated);
      return updated;
    });
    return conversation;
  }, [vmId]);

  const updateConversation = useCallback((id: string, update: Partial<Conversation>) => {
    setConversations((prev) => {
      const updated = prev.map((c) => (c.conversationId === id ? { ...c, ...update } : c));
      saveConversationsToStorage(vmId, updated);
      return updated;
    });
  }, [vmId]);

  const deleteConversation = useCallback((id: string) => {
    setConversations((prev) => {
      const updated = prev.filter((c) => c.conversationId !== id);
      saveConversationsToStorage(vmId, updated);
      return updated;
    });
  }, [vmId]);

  const loadHistory = useCallback(async (): Promise<ChatSession[]> => {
    const res = await fetch("/chat-history");
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
  }, []);

  const syncConversationsFromHistory = useCallback(async () => {
    const sessions = await loadHistory();
    setConversations((prev) => {
      const existingSessionIds = new Set(
        prev.map((c) => c.sessionId).filter((id): id is string => !!id),
      );
      const newConversations: Conversation[] = sessions
        .filter((s) => !existingSessionIds.has(s.session_id))
        .map((s) => ({
          conversationId: crypto.randomUUID(),
          sessionId: s.session_id,
          projectDir: s.project_dir,
          title: s.title,
          createdAt: new Date(s.created_at).getTime(),
        }));
      if (newConversations.length === 0) return prev;
      const updated = [...prev, ...newConversations];
      saveConversationsToStorage(vmId, updated);
      return updated;
    });
  }, [loadHistory, vmId]);

  // On mount: sync server sessions into local conversations
  useEffect(() => {
    syncConversationsFromHistory().catch(console.error);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const esRef = useRef<EventSource | null>(null);

  // On mount: check for in-progress task and open reconnect stream
  useEffect(() => {
    const storageKey = `chat_running_task_${vmId}`;
    const saved = localStorage.getItem(storageKey);
    if (!saved) return;

    let parsed: { task_id?: string; running_session_id?: string | null };
    try {
      parsed = JSON.parse(saved) as { task_id?: string; running_session_id?: string | null };
    } catch {
      localStorage.removeItem(storageKey);
      return;
    }

    if (!parsed.task_id) {
      localStorage.removeItem(storageKey);
      return;
    }

    const taskId = parsed.task_id;
    const conversationId = parsed.running_session_id ?? taskId;

    pushEvent({
      type: "reconnecting",
      payload: { task_id: taskId, conversation_id: conversationId },
    });

    const url = `/chat-stream/${encodeURIComponent(taskId)}?conversation_id=${encodeURIComponent(conversationId)}`;
    const es = new EventSource(url);
    esRef.current = es;
    attachEventSourceListeners(es, pushEvent, vmId);

    return () => {
      es.close();
      esRef.current = null;
    };
  }, [vmId, pushEvent]);

  const sendQuery = useCallback((
    content: string,
    conversationId: string,
    sessionId?: string,
    workDir?: string,
  ) => {
    const executeStream = async () => {
      const res = await fetch("/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          conversation_id: conversationId,
          content,
          session_id: sessionId ?? null,
          work_dir: workDir ?? null,
          csrf_token: csrfTokenRef.current,
        }),
      });
      if (!res.ok) {
        const msg = await res.text();
        throw new Error(msg || `HTTP ${res.status}`);
      }
      refreshCsrfToken(res);
      await readFetchSseStream(res, pushEvent, vmId);
    };
    executeStream().catch((err: unknown) => {
      pushEvent({ type: "error_event", payload: { message: String(err) } });
    });
  }, [vmId, pushEvent, refreshCsrfToken]);

  const post = useCallback(async (path: string, body: Record<string, unknown>) => {
    const res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ...body, csrf_token: csrfTokenRef.current }),
    });
    if (!res.ok) {
      const msg = await res.text();
      throw new Error(msg || `HTTP ${res.status}`);
    }
    refreshCsrfToken(res);
  }, [refreshCsrfToken]);

  const sendStop = useCallback(async (taskId: string) => {
    await post("/chat-stop", { task_id: taskId });
  }, [post]);

  const answerQuestion = useCallback(async (taskId: string, requestId: string, answers: Record<string, string>) => {
    await post("/chat-question-answer", { task_id: taskId, request_id: requestId, answers });
  }, [post]);

  const loadTranscript = useCallback(async (
    sessionId: string,
    projectDir: string,
    signal?: AbortSignal,
  ): Promise<TranscriptMessage[]> => {
    const params = new URLSearchParams({ session_id: sessionId, project_dir: projectDir });
    const res = await fetch(`/chat-transcript?${params}`, { signal });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    return data.messages as TranscriptMessage[];
  }, []);

  const deleteSession = useCallback(async (sessionId: string, projectDir: string) => {
    const res = await fetch("/chat-transcript", {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ csrf_token: csrfTokenRef.current, session_id: sessionId, project_dir: projectDir }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    refreshCsrfToken(res);
  }, [refreshCsrfToken]);

  const listFiles = useCallback(async (path: string): Promise<FileEntry[]> => {
    const res = await fetch(`/ls?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    return data.entries as FileEntry[];
  }, []);

  const storeQuestion = useCallback((requestId: string, data: StoredQuestion) => {
    localStorage.setItem(`question_${requestId}`, JSON.stringify(data));
  }, []);

  const clearQuestion = useCallback((requestId: string) => {
    localStorage.removeItem(`question_${requestId}`);
  }, []);

  const getQuestionsForConversation = useCallback((conversationId: string): StoredQuestion | null => {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key?.startsWith("question_")) {
        try {
          const data = JSON.parse(localStorage.getItem(key)!) as StoredQuestion;
          if (data.conversationId === conversationId) {
            return data;
          }
        } catch { /* ignore */ }
      }
    }
    return null;
  }, []);

  return (
    <SseContext.Provider value={{
      vmId,
      csrfToken,
      uploadDir,
      uploadAction,
      hasUserRootfs,
      eventQueueRef,
      eventSeq,
      conversations,
      createConversation,
      updateConversation,
      deleteConversation,
      syncConversationsFromHistory,
      sendQuery,
      sendStop,
      answerQuestion,
      loadHistory,
      loadTranscript,
      deleteSession,
      listFiles,
      storeQuestion,
      clearQuestion,
      getQuestionsForConversation,
    }}>
      {children}
    </SseContext.Provider>
  );
}

export function useSse(): SseContextValue {
  const ctx = useContext(SseContext);
  if (!ctx) throw new Error("useSse must be used inside SseProvider");
  return ctx;
}

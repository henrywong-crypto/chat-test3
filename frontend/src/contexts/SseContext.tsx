import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ChatSession, FileEntry, SseEvent, TranscriptMessage } from "../types";

interface SseContextValue {
  vmId: string;
  csrfToken: string;
  uploadDir: string;
  uploadAction: string;
  hasUserRootfs: boolean;
  latestEvent: SseEvent | null;
  isConnected: boolean;
  sendQuery: (content: string, sessionId: string | null) => Promise<void>;
  sendStop: () => Promise<void>;
  answerQuestion: (requestId: string, answers: Record<string, string>) => Promise<void>;
  loadHistory: () => Promise<ChatSession[]>;
  loadTranscript: (sessionId: string, projectDir: string, signal?: AbortSignal) => Promise<TranscriptMessage[]>;
  deleteSession: (sessionId: string, projectDir: string) => Promise<void>;
  listFiles: (path: string) => Promise<FileEntry[]>;
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

export function SseProvider({ children }: { children: React.ReactNode }) {
  const config = useRef(readAppConfig());
  const { vmId, csrfToken, uploadDir, uploadAction, hasUserRootfs } = config.current;

  const [latestEvent, setLatestEvent] = useState<SseEvent | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    const es = new EventSource(`/sessions/${vmId}/chat-stream`);
    esRef.current = es;

    es.onopen = () => setIsConnected(true);

    es.onerror = () => {
      if (es.readyState === EventSource.CLOSED) {
        setIsConnected(false);
        esRef.current = null;
      }
    };

    const addListener = (eventType: string, handler: (e: MessageEvent) => void) => {
      es.addEventListener(eventType, handler as EventListener);
    };

    addListener("init", () => {
      setLatestEvent({ type: "init" });
    });

    addListener("text_delta", (e) => {
      setLatestEvent({ type: "text_delta", payload: JSON.parse(e.data) });
    });

    addListener("thinking_delta", (e) => {
      setLatestEvent({ type: "thinking_delta", payload: JSON.parse(e.data) });
    });

    addListener("tool_start", (e) => {
      setLatestEvent({ type: "tool_start", payload: JSON.parse(e.data) });
    });

    addListener("ask_user_question", (e) => {
      setLatestEvent({ type: "ask_user_question", payload: JSON.parse(e.data) });
    });

    addListener("tool_result", (e) => {
      setLatestEvent({ type: "tool_result", payload: JSON.parse(e.data) });
    });

    addListener("done", (e) => {
      setLatestEvent({ type: "done", payload: JSON.parse(e.data) });
    });

    addListener("error_event", (e) => {
      setLatestEvent({ type: "error_event", payload: JSON.parse(e.data) });
    });

    return () => {
      es.close();
    };
  }, [vmId]);

  const post = useCallback(async (path: string, body: Record<string, unknown>) => {
    const res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ...body, csrf_token: csrfToken }),
    });
    if (!res.ok) {
      const msg = await res.text();
      throw new Error(msg || `HTTP ${res.status}`);
    }
  }, [csrfToken]);

  const sendQuery = useCallback(async (content: string, sessionId: string | null) => {
    await post(`/sessions/${vmId}/chat`, { content, session_id: sessionId });
  }, [post, vmId]);

  const sendStop = useCallback(async () => {
    await post(`/sessions/${vmId}/chat-stop`, {});
  }, [post, vmId]);

  const answerQuestion = useCallback(async (requestId: string, answers: Record<string, string>) => {
    await post(`/sessions/${vmId}/chat-question-answer`, { request_id: requestId, answers });
  }, [post, vmId]);

  const loadHistory = useCallback(async (): Promise<ChatSession[]> => {
    const res = await fetch(`/sessions/${vmId}/chat-history`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
  }, [vmId]);

  const loadTranscript = useCallback(async (
    sessionId: string,
    projectDir: string,
    signal?: AbortSignal,
  ): Promise<TranscriptMessage[]> => {
    const params = new URLSearchParams({ session_id: sessionId, project_dir: projectDir });
    const res = await fetch(`/sessions/${vmId}/chat-transcript?${params}`, { signal });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    return data.messages as TranscriptMessage[];
  }, [vmId]);

  const deleteSession = useCallback(async (sessionId: string, projectDir: string) => {
    const res = await fetch(`/sessions/${vmId}/chat-transcript`, {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ csrf_token: csrfToken, session_id: sessionId, project_dir: projectDir }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
  }, [vmId, csrfToken]);

  const listFiles = useCallback(async (path: string): Promise<FileEntry[]> => {
    const res = await fetch(`/sessions/${vmId}/ls?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    return data.entries as FileEntry[];
  }, [vmId]);

  return (
    <SseContext.Provider value={{
      vmId,
      csrfToken,
      uploadDir,
      uploadAction,
      hasUserRootfs,
      latestEvent,
      isConnected,
      sendQuery,
      sendStop,
      answerQuestion,
      loadHistory,
      loadTranscript,
      deleteSession,
      listFiles,
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

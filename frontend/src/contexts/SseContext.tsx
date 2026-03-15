import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ChatSession, FileEntry, SseEvent, TranscriptMessage } from "../types";

interface SseContextValue {
  vmId: string;
  csrfToken: string;
  uploadDir: string;
  uploadAction: string;
  hasUserRootfs: boolean;
  eventQueueRef: React.MutableRefObject<SseEvent[]>;
  eventSeq: number;
  isConnected: boolean;
  isVmReady: boolean;
  sendQuery: (content: string, sessionId: string | null, workDir?: string) => Promise<string>;
  sendStop: (taskId: string) => Promise<void>;
  answerQuestion: (taskId: string, requestId: string, answers: Record<string, string>) => Promise<void>;
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
  const { vmId, uploadDir, uploadAction, hasUserRootfs } = config.current;

  // csrfToken is rotated by the server after every validated POST/DELETE.
  // Keep a ref for synchronous access in callbacks and state for reactive consumers.
  const csrfTokenRef = useRef(config.current.csrfToken);
  const [csrfToken, setCsrfToken] = useState(config.current.csrfToken);

  const refreshCsrfToken = useCallback((res: Response) => {
    const newToken = res.headers.get("x-csrf-token");
    if (newToken) {
      csrfTokenRef.current = newToken;
      setCsrfToken(newToken);
    }
  }, []);

  // Events are queued in a ref so none are lost when React batches renders.
  // eventSeq is a plain counter that increments each time an event is enqueued,
  // signalling useSseHandlers to drain the queue. React 18 auto-batches the
  // setState calls from the EventSource listeners, so multiple rapid events
  // are processed together without flushSync.
  const eventQueueRef = useRef<SseEvent[]>([]);
  const [eventSeq, setEventSeq] = useState(0);

  const pushEvent = useCallback((event: SseEvent) => {
    eventQueueRef.current.push(event);
    setEventSeq((s) => s + 1);
  }, []);

  const [isConnected, setIsConnected] = useState(false);
  const [isVmReady, setIsVmReady] = useState(false);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    const es = new EventSource(`/sessions/${vmId}/chat-stream`);
    esRef.current = es;

    es.onopen = () => {
      setIsConnected(true);
      const storageKey = `chat_running_task_${vmId}`;
      const saved = localStorage.getItem(storageKey);
      if (saved) {
        try {
          const parsed = JSON.parse(saved) as { task_id?: string; running_session_id?: string | null; project_dir?: string | null };
          if (parsed.task_id) {
            fetch(`/sessions/${vmId}/chat-hello`, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ task_id: parsed.task_id, csrf_token: csrfTokenRef.current }),
            }).catch(console.error);
            pushEvent({
              type: "reconnecting",
              payload: {
                task_id: parsed.task_id!,
                running_session_id: parsed.running_session_id ?? null,
                project_dir: parsed.project_dir ?? null,
              },
            });
          } else {
            localStorage.removeItem(storageKey);
          }
        } catch {
          localStorage.removeItem(storageKey);
        }
      }
    };

    es.onerror = () => {
      setIsVmReady(false);
      if (es.readyState === EventSource.CLOSED) {
        setIsConnected(false);
        esRef.current = null;
      }
    };

    const addListener = (eventType: string, handler: (e: MessageEvent) => void) => {
      es.addEventListener(eventType, handler as EventListener);
    };

    addListener("relay_ready", () => {
      setIsVmReady(true);
    });

    addListener("session_start", (e) => {
      pushEvent({ type: "session_start", payload: JSON.parse(e.data) });
    });

    addListener("init", () => {
      pushEvent({ type: "init" });
    });

    addListener("text_delta", (e) => {
      pushEvent({ type: "text_delta", payload: JSON.parse(e.data) });
    });

    addListener("thinking_delta", (e) => {
      pushEvent({ type: "thinking_delta", payload: JSON.parse(e.data) });
    });

    addListener("tool_start", (e) => {
      pushEvent({ type: "tool_start", payload: JSON.parse(e.data) });
    });

    addListener("ask_user_question", (e) => {
      pushEvent({ type: "ask_user_question", payload: JSON.parse(e.data) });
    });

    addListener("tool_result", (e) => {
      pushEvent({ type: "tool_result", payload: JSON.parse(e.data) });
    });

    addListener("done", (e) => {
      localStorage.removeItem(`chat_running_task_${vmId}`);
      pushEvent({ type: "done", payload: JSON.parse(e.data) });
    });

    addListener("error_event", (e) => {
      localStorage.removeItem(`chat_running_task_${vmId}`);
      pushEvent({ type: "error_event", payload: JSON.parse(e.data) });
    });

    return () => {
      es.close();
      setIsVmReady(false);
    };
  }, [vmId, pushEvent]);

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

  const sendQuery = useCallback(async (content: string, sessionId: string | null, workDir?: string): Promise<string> => {
    const res = await fetch(`/sessions/${vmId}/chat`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ content, session_id: sessionId, work_dir: workDir ?? null, csrf_token: csrfTokenRef.current }),
    });
    if (!res.ok) {
      const msg = await res.text();
      throw new Error(msg || `HTTP ${res.status}`);
    }
    refreshCsrfToken(res);
    const { task_id } = await res.json() as { task_id: string };
    return task_id;
  }, [vmId, refreshCsrfToken]);

  const sendStop = useCallback(async (taskId: string) => {
    await post(`/sessions/${vmId}/chat-stop`, { task_id: taskId });
  }, [post, vmId]);

  const answerQuestion = useCallback(async (taskId: string, requestId: string, answers: Record<string, string>) => {
    await post(`/sessions/${vmId}/chat-question-answer`, { task_id: taskId, request_id: requestId, answers });
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
      body: JSON.stringify({ csrf_token: csrfTokenRef.current, session_id: sessionId, project_dir: projectDir }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    refreshCsrfToken(res);
  }, [vmId, refreshCsrfToken]);

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
      eventQueueRef,
      eventSeq,
      isConnected,
      isVmReady,
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

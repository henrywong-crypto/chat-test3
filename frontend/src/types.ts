export interface ChatSession {
  session_id: string;
  created_at: string;
  title: string;
  project_dir?: string;
}

export interface TranscriptMessage {
  role: string;
  content: string | ContentBlock[];
  isCompactSummary: boolean;
}

export interface ContentBlock {
  type: string;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  thinking?: string;
  tool_use_id?: string;
  content?: string | ContentBlock[];
  is_error?: boolean;
}

export interface Question {
  question: string;
  header?: string;
  options: QuestionOption[];
  multiSelect?: boolean;
}

export interface QuestionOption {
  label: string;
  description?: string;
}

export interface PendingQuestion {
  requestId: string;
  taskId: string;
  questions: Question[];
}

export interface ToolResult {
  content: string;
  isError: boolean;
}

export interface ChatMessage {
  id: string;
  type: "user" | "assistant" | "tool" | "error";
  content: string;
  timestamp: number;
  isThinking?: boolean;
  isToolUse?: boolean;
  toolId?: string;
  toolName?: string;
  toolInput?: Record<string, unknown>;
  toolResult?: ToolResult;
}

// SSE event payloads
export interface SseTextDelta {
  text: string;
}

export interface SseThinkingDelta {
  thinking: string;
}

export interface SseToolStart {
  id: string;
  name: string;
  input: Record<string, unknown>;
}

export interface SseSessionStart {
  task_id: string;
}

export interface SseAskUserQuestion {
  request_id: string;
  task_id: string;
  questions: Question[];
}

export interface SseToolResult {
  tool_use_id: string;
  content: string;
  is_error: boolean;
}

export interface SseDone {
  session_id: string | null;
  task_id: string;
}

export interface SseErrorEvent {
  message: string;
}

export interface SseReconnecting {
  task_id: string;
  running_session_id: string | null;
  project_dir: string | null;
}

export type SseEvent =
  | { type: "relay_ready" }
  | { type: "session_start"; payload: SseSessionStart }
  | { type: "init" }
  | { type: "text_delta"; payload: SseTextDelta }
  | { type: "thinking_delta"; payload: SseThinkingDelta }
  | { type: "tool_start"; payload: SseToolStart }
  | { type: "ask_user_question"; payload: SseAskUserQuestion }
  | { type: "tool_result"; payload: SseToolResult }
  | { type: "done"; payload: SseDone }
  | { type: "error_event"; payload: SseErrorEvent }
  | { type: "reconnecting"; payload: SseReconnecting };

export interface FileEntry {
  name: string;
  is_dir: boolean;
  size: number;
}

export type ViewTab = "chat" | "terminal" | "files";

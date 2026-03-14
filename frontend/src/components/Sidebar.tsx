import React from "react";
import { Plus, RefreshCw, Trash2 } from "lucide-react";
import type { ChatSession } from "../types";

interface SidebarProps {
  sessions: ChatSession[];
  viewSessionId: string | null;
  runningSessionId: string | null;
  onSelectSession: (session: ChatSession) => void;
  onNewChat: () => void;
  onRefresh: () => void;
  onDeleteSession: (session: ChatSession) => void;
}

export default function Sidebar({
  sessions,
  viewSessionId,
  runningSessionId,
  onSelectSession,
  onNewChat,
  onRefresh,
  onDeleteSession,
}: SidebarProps) {
  return (
    <div className="flex w-64 flex-col border-r border-border bg-card">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-semibold text-foreground">Chats</span>
        <button
          title="Refresh"
          onClick={onRefresh}
          className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <RefreshCw className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto py-1">
        {sessions.length === 0 ? (
          <div className="px-3 py-4 text-center text-xs text-muted-foreground">No chats yet</div>
        ) : (
          sessions.map((session) => (
            <SessionRow
              key={session.session_id}
              session={session}
              isActive={session.session_id === viewSessionId}
              isRunning={session.session_id === runningSessionId}
              onSelect={() => onSelectSession(session)}
              onDelete={() => onDeleteSession(session)}
            />
          ))
        )}
      </div>

      <div className="border-t border-border px-2 py-2">
        <button
          onClick={onNewChat}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-primary/10 px-3 py-2 text-sm font-medium text-primary transition-colors hover:bg-primary/20"
        >
          <Plus className="h-4 w-4" />
          New Chat
        </button>
      </div>
    </div>
  );
}

function SessionRow({
  session,
  isActive,
  isRunning,
  onSelect,
  onDelete,
}: {
  session: ChatSession;
  isActive: boolean;
  isRunning: boolean;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const [hovered, setHovered] = React.useState(false);

  const title = session.title || `Session ${session.session_id.slice(0, 8)}`;

  return (
    <div
      className={`group relative flex cursor-pointer items-center gap-2 px-3 py-2 transition-colors ${
        isActive
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"
      }`}
      onClick={onSelect}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {isRunning && (
        <span className="flex h-1.5 w-1.5 flex-shrink-0">
          <span className="absolute inline-flex h-1.5 w-1.5 animate-ping rounded-full bg-primary opacity-75" />
          <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-primary" />
        </span>
      )}
      <span className="flex-1 truncate text-xs">{title}</span>
      {hovered && (
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:text-destructive"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}

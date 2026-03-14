import React from "react";
import { ChevronDown, ChevronRight, Wrench } from "lucide-react";
import type { ToolResult } from "../types";
import ToolDiffViewer from "./ToolDiffViewer";

interface ToolRendererProps {
  toolName: string;
  toolInput: Record<string, unknown>;
  toolResult?: ToolResult;
}

export default function ToolRenderer({ toolName, toolInput, toolResult }: ToolRendererProps) {
  return (
    <div className="my-1 rounded-lg border border-border/50 bg-card/80 backdrop-blur-sm">
      <ToolHeader toolName={toolName} toolInput={toolInput} toolResult={toolResult} />
    </div>
  );
}

type DiffBadge = "Edit" | "New" | "Patch";

function isEditTool(toolName: string): boolean {
  return toolName === "Edit" || toolName === "Write" || toolName === "ApplyPatch";
}

function getDiffProps(toolName: string, input: Record<string, unknown>): { oldContent: string; newContent: string; filePath: string; badge: DiffBadge } | null {
  if (toolName === "Edit") {
    const filePath = String(input.file_path ?? "");
    const oldContent = String(input.old_string ?? "");
    const newContent = String(input.new_string ?? "");
    return { oldContent, newContent, filePath, badge: "Edit" };
  }
  if (toolName === "Write") {
    const filePath = String(input.file_path ?? "");
    const newContent = String(input.content ?? "");
    return { oldContent: "", newContent, filePath, badge: "New" };
  }
  if (toolName === "ApplyPatch") {
    const filePath = String(input.file_path ?? input.path ?? "");
    const oldContent = String(input.old ?? input.original ?? "");
    const newContent = String(input.new ?? input.patched ?? "");
    return { oldContent, newContent, filePath, badge: "Patch" };
  }
  return null;
}

function ToolHeader({
  toolName,
  toolInput,
  toolResult,
}: {
  toolName: string;
  toolInput: Record<string, unknown>;
  toolResult?: ToolResult;
}) {
  const diffProps = isEditTool(toolName) ? getDiffProps(toolName, toolInput) : null;
  const [open, setOpen] = React.useState(diffProps !== null);

  const summary = buildSummary(toolName, toolInput);

  return (
    <div>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-accent/40 active:bg-accent/60"
      >
        <Wrench className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
        <span className="flex-1 truncate text-xs font-medium text-foreground">
          <span className="text-muted-foreground">{toolName}</span>
          {summary && <span className="ml-2 text-foreground/70">{summary}</span>}
        </span>
        {open ? (
          <ChevronDown className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
        )}
      </button>
      {open && diffProps && (
        <ToolDiffViewer
          oldContent={diffProps.oldContent}
          newContent={diffProps.newContent}
          filePath={diffProps.filePath}
          badge={diffProps.badge}
        />
      )}
      {open && !diffProps && (
        <div className="border-t border-border px-3 py-2">
          <ToolInputBody toolName={toolName} toolInput={toolInput} />
        </div>
      )}
      {open && toolResult && !isEditTool(toolName) && <ToolResultView result={toolResult} />}
      {open && toolResult?.isError && isEditTool(toolName) && <ToolResultView result={toolResult} />}
    </div>
  );
}

function ToolInputBody({ toolName, toolInput }: { toolName: string; toolInput: Record<string, unknown> }) {
  if (toolName === "Bash" || toolName === "shell") {
    return <BashInputBody toolInput={toolInput} />;
  }
  if (toolName === "Grep") {
    return <GrepInputBody toolInput={toolInput} />;
  }
  if (toolName === "Glob") {
    return <GlobInputBody toolInput={toolInput} />;
  }
  if (toolName === "WebFetch") {
    return <WebFetchInputBody toolInput={toolInput} />;
  }
  if (toolName === "WebSearch") {
    return <WebSearchInputBody toolInput={toolInput} />;
  }
  if (toolName === "TodoWrite" || toolName === "TodoRead") {
    return <TodoInputBody toolInput={toolInput} />;
  }
  return (
    <pre className="overflow-x-auto text-xs text-muted-foreground">
      {JSON.stringify(toolInput, null, 2)}
    </pre>
  );
}

function BashInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const cmd = toolInput.command ?? toolInput.cmd;
  const desc = toolInput.description;
  return (
    <div>
      {typeof cmd === "string" && (
        <pre className="overflow-x-auto whitespace-pre-wrap break-all text-xs text-foreground/80">{cmd}</pre>
      )}
      {typeof desc === "string" && (
        <p className="mt-1 text-[11px] text-muted-foreground">{desc}</p>
      )}
    </div>
  );
}

function GrepInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const pattern = toolInput.pattern;
  const path = toolInput.path ?? toolInput.glob;
  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      {typeof pattern === "string" && (
        <code className="rounded bg-muted px-1.5 py-0.5 text-foreground/80">/{pattern}/</code>
      )}
      {typeof path === "string" && (
        <span className="text-muted-foreground">{path}</span>
      )}
    </div>
  );
}

function GlobInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const pattern = toolInput.pattern;
  const path = toolInput.path;
  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      {typeof pattern === "string" && (
        <code className="rounded bg-muted px-1.5 py-0.5 text-foreground/80">{pattern}</code>
      )}
      {typeof path === "string" && (
        <span className="text-muted-foreground">{path}</span>
      )}
    </div>
  );
}

function WebFetchInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const url = toolInput.url;
  return typeof url === "string" ? (
    <span className="break-all text-xs text-muted-foreground">{url}</span>
  ) : null;
}

function WebSearchInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const query = toolInput.query;
  return typeof query === "string" ? (
    <span className="text-xs text-muted-foreground">{query}</span>
  ) : null;
}

function TodoInputBody({ toolInput }: { toolInput: Record<string, unknown> }) {
  const todos = toolInput.todos;
  if (Array.isArray(todos)) {
    return (
      <ul className="space-y-0.5 text-xs text-muted-foreground">
        {todos.slice(0, 5).map((todo, i) => (
          <li key={i} className="truncate">
            {typeof todo === "object" && todo !== null
              ? String((todo as Record<string, unknown>).content ?? JSON.stringify(todo))
              : String(todo)}
          </li>
        ))}
        {todos.length > 5 && <li className="text-muted-foreground/60">+{todos.length - 5} more</li>}
      </ul>
    );
  }
  return (
    <pre className="overflow-x-auto text-xs text-muted-foreground">
      {JSON.stringify(toolInput, null, 2)}
    </pre>
  );
}

function ToolResultView({ result }: { result: ToolResult }) {
  const [open, setOpen] = React.useState(false);
  const isLong = result.content.length > 200;

  return (
    <div className={`border-t border-border px-3 py-2 ${result.isError ? "bg-red-950/10" : ""}`}>
      {result.isError && (
        <div className="mb-1 text-[10px] font-medium uppercase tracking-wide text-red-400">Error</div>
      )}
      {isLong ? (
        <div>
          <div className="relative overflow-hidden">
            <pre
              className={`whitespace-pre-wrap break-words text-xs ${
                result.isError ? "text-red-300" : "text-muted-foreground"
              } ${!open ? "max-h-24" : ""}`}
              style={{ overflow: open ? "auto" : "hidden" }}
            >
              {result.content}
            </pre>
            {!open && (
              <div className="absolute bottom-0 left-0 right-0 h-8 bg-gradient-to-t from-card/80 to-transparent" />
            )}
          </div>
          <button
            onClick={() => setOpen((v) => !v)}
            className="mt-1 text-[10px] text-primary hover:underline"
          >
            {open ? "Show less" : "Show more"}
          </button>
        </div>
      ) : (
        <pre
          className={`whitespace-pre-wrap break-words text-xs ${
            result.isError ? "text-red-300" : "text-muted-foreground"
          }`}
        >
          {result.content}
        </pre>
      )}
    </div>
  );
}

function buildSummary(toolName: string, input: Record<string, unknown>): string {
  if (toolName === "Bash" || toolName === "shell") {
    const cmd = input.command ?? input.cmd;
    if (typeof cmd === "string") return cmd.slice(0, 80);
  }
  if (toolName === "Read" || toolName === "Write" || toolName === "Edit" || toolName === "Glob") {
    const path = input.file_path ?? input.path ?? input.pattern;
    if (typeof path === "string") return path.slice(0, 80);
  }
  if (toolName === "Grep") {
    const pattern = input.pattern;
    if (typeof pattern === "string") return `/${pattern}/`;
  }
  if (toolName === "WebFetch") {
    const url = input.url;
    if (typeof url === "string") return url.slice(0, 80);
  }
  if (toolName === "WebSearch") {
    const query = input.query;
    if (typeof query === "string") return query.slice(0, 80);
  }
  return "";
}

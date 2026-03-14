import React from "react";
import { ChevronDown, ChevronRight, Wrench } from "lucide-react";
import type { ToolResult } from "../types";

interface ToolRendererProps {
  toolName: string;
  toolInput: Record<string, unknown>;
  toolResult?: ToolResult;
}

export default function ToolRenderer({ toolName, toolInput, toolResult }: ToolRendererProps) {
  return (
    <div className="my-1 rounded-lg border border-border bg-card/50">
      <ToolHeader toolName={toolName} toolInput={toolInput} />
      {toolResult && <ToolResultView result={toolResult} />}
    </div>
  );
}

function ToolHeader({ toolName, toolInput }: { toolName: string; toolInput: Record<string, unknown> }) {
  const [open, setOpen] = React.useState(false);

  const summary = buildSummary(toolName, toolInput);

  return (
    <div>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-accent/40"
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
      {open && (
        <div className="border-t border-border px-3 py-2">
          <pre className="overflow-x-auto text-xs text-muted-foreground">
            {JSON.stringify(toolInput, null, 2)}
          </pre>
        </div>
      )}
    </div>
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
  return "";
}

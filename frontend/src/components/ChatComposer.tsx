import React, { useCallback, useRef, useState } from "react";
import { Send, Square } from "lucide-react";

interface ChatComposerProps {
  isLoading: boolean;
  onSend: (text: string) => void;
  onStop: () => void;
}

interface SlashCommand {
  name: string;
  description: string;
}

const SLASH_COMMANDS: SlashCommand[] = [
  { name: "/help", description: "Show help and available commands" },
  { name: "/clear", description: "Clear conversation history" },
  { name: "/compact", description: "Compact conversation with optional instructions" },
  { name: "/config", description: "Open config panel" },
  { name: "/cost", description: "Show token usage and cost" },
  { name: "/doctor", description: "Check Claude Code installation health" },
  { name: "/init", description: "Initialize project with CLAUDE.md" },
  { name: "/login", description: "Switch Anthropic accounts" },
  { name: "/logout", description: "Log out" },
  { name: "/memory", description: "Edit memory files" },
  { name: "/mcp", description: "Manage MCP servers" },
  { name: "/model", description: "Set or switch model" },
  { name: "/pr_comments", description: "Get PR comments" },
  { name: "/review", description: "Request code review" },
  { name: "/status", description: "Show account/model status" },
  { name: "/terminal", description: "Run shell command" },
  { name: "/vim", description: "Enter vim mode" },
];

export default function ChatComposer({ isLoading, onSend, onStop }: ChatComposerProps) {
  const [input, setInput] = useState("");
  const [slashMenuOpen, setSlashMenuOpen] = useState(false);
  const [slashMenuIndex, setSlashMenuIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const filteredCommands = input.startsWith("/")
    ? SLASH_COMMANDS.filter((cmd) =>
        cmd.name.startsWith(input.split(" ")[0].toLowerCase())
      )
    : [];

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || isLoading) return;
    setInput("");
    setSlashMenuOpen(false);
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
    onSend(text);
  }, [input, isLoading, onSend]);

  const selectCommand = useCallback(
    (cmd: SlashCommand) => {
      setInput(cmd.name + " ");
      setSlashMenuOpen(false);
      setSlashMenuIndex(0);
      textareaRef.current?.focus();
    },
    [],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (slashMenuOpen && filteredCommands.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSlashMenuIndex((i) => (i + 1) % filteredCommands.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSlashMenuIndex((i) => (i - 1 + filteredCommands.length) % filteredCommands.length);
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          selectCommand(filteredCommands[slashMenuIndex]);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setSlashMenuOpen(false);
          return;
        }
      }

      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [slashMenuOpen, filteredCommands, slashMenuIndex, selectCommand, handleSend],
  );

  const handleInput = useCallback((e: React.FormEvent<HTMLTextAreaElement>) => {
    const target = e.target as HTMLTextAreaElement;
    target.style.height = "auto";
    target.style.height = Math.min(target.scrollHeight, 300) + "px";
    const value = target.value;
    setInput(value);
    setSlashMenuIndex(0);
    if (value.startsWith("/") && !value.includes(" ")) {
      setSlashMenuOpen(true);
    } else {
      setSlashMenuOpen(false);
    }
  }, []);

  const menuVisible = slashMenuOpen && filteredCommands.length > 0;

  return (
    <div className="flex-shrink-0 border-t border-border/50 bg-card/50 p-2 pb-2 sm:p-4 sm:pb-4">
      <div className="mx-auto max-w-3xl">
        <div className="relative">
          {menuVisible && (
            <div className="absolute bottom-full left-0 right-0 mb-1 overflow-hidden rounded-xl border border-border bg-card shadow-lg">
              {filteredCommands.map((cmd, i) => (
                <button
                  key={cmd.name}
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    selectCommand(cmd);
                  }}
                  className={`flex w-full items-baseline gap-3 px-3 py-2 text-left transition-colors ${
                    i === slashMenuIndex ? "bg-accent" : "hover:bg-accent/50"
                  }`}
                >
                  <span className="font-mono text-xs font-medium text-foreground">{cmd.name}</span>
                  <span className="truncate text-[11px] text-muted-foreground">{cmd.description}</span>
                </button>
              ))}
            </div>
          )}
          <div className="relative flex items-end gap-2 rounded-2xl border border-border/50 bg-card/80 px-3 py-2 shadow-sm backdrop-blur-sm transition-all duration-200 focus-within:border-primary/30 focus-within:shadow-md focus-within:ring-1 focus-within:ring-primary/15">
            <textarea
              ref={textareaRef}
              value={input}
              onInput={handleInput}
              onKeyDown={handleKeyDown}
              onChange={(e) => setInput(e.target.value)}
              placeholder="Message Claude…"
              disabled={isLoading}
              rows={1}
              className="max-h-[300px] min-h-[36px] flex-1 resize-none bg-transparent py-1 text-sm text-foreground placeholder-muted-foreground/60 focus:outline-none disabled:opacity-60"
              style={{ height: "36px" }}
            />
            {isLoading ? (
              <button
                type="button"
                onClick={onStop}
                title="Stop (Esc)"
                className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl bg-destructive text-destructive-foreground transition-all hover:bg-destructive/90"
              >
                <Square className="h-4 w-4" />
              </button>
            ) : (
              <button
                type="button"
                onClick={handleSend}
                disabled={!input.trim()}
                title="Send"
                className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground transition-all hover:bg-primary/90 disabled:bg-muted disabled:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/30 focus:ring-offset-1 focus:ring-offset-background"
              >
                <Send className="h-4 w-4" />
              </button>
            )}
          </div>
        </div>
        <p className="mt-1.5 text-center text-[10px] text-muted-foreground/50">
          Enter to send · Shift+Enter for newline
        </p>
      </div>
    </div>
  );
}

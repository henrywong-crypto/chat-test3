import React, { useCallback, useRef, useState } from "react";
import { Send, Square } from "lucide-react";

interface ChatComposerProps {
  isLoading: boolean;
  onSend: (text: string) => void;
  onStop: () => void;
}

export default function ChatComposer({ isLoading, onSend, onStop }: ChatComposerProps) {
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || isLoading) return;
    setInput("");
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
    onSend(text);
  }, [input, isLoading, onSend]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleInput = useCallback((e: React.FormEvent<HTMLTextAreaElement>) => {
    const target = e.target as HTMLTextAreaElement;
    target.style.height = "auto";
    target.style.height = Math.min(target.scrollHeight, 300) + "px";
    setInput(target.value);
  }, []);

  return (
    <div className="flex-shrink-0 border-t border-border bg-card p-3">
      <div className="mx-auto max-w-3xl">
        <div className="relative flex items-end gap-2 rounded-2xl border border-border bg-background px-3 py-2 focus-within:border-primary/50 focus-within:ring-1 focus-within:ring-primary/20">
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
              className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground transition-all hover:bg-primary/90 disabled:bg-muted disabled:text-muted-foreground"
            >
              <Send className="h-4 w-4" />
            </button>
          )}
        </div>
        <p className="mt-1.5 text-center text-[10px] text-muted-foreground/50">
          Enter to send · Shift+Enter for newline
        </p>
      </div>
    </div>
  );
}

import React, { useEffect, useRef } from "react";
import type { ChatMessage } from "../types";
import MessageComponent from "./MessageComponent";

interface ChatMessagesPaneProps {
  messages: ChatMessage[];
  isLoading: boolean;
}

export default function ChatMessagesPane({ messages, isLoading }: ChatMessagesPaneProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);

  // Auto-scroll on new messages unless user has scrolled up
  useEffect(() => {
    if (userScrolledRef.current) return;
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  });

  const handleWheel = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    userScrolledRef.current = !atBottom;
  };

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) userScrolledRef.current = false;
  };

  if (messages.length === 0 && !isLoading) {
    return (
      <div
        ref={scrollRef}
        className="flex flex-1 items-center justify-center overflow-y-auto"
      >
        <div className="text-center text-muted-foreground">
          <div className="mb-3 text-4xl opacity-20">💬</div>
          <p className="text-sm">Start a new conversation</p>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={scrollRef}
      onWheel={handleWheel}
      onScroll={handleScroll}
      className="flex-1 space-y-1 overflow-y-auto py-4"
    >
      {messages.map((message, index) => {
        const prevMessage = index > 0 ? messages[index - 1] : null;
        return (
          <MessageComponent
            key={message.id}
            message={message}
            prevMessage={prevMessage}
          />
        );
      })}
    </div>
  );
}

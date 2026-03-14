import React, { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import type { ChatMessage } from "../types";
import MessageComponent from "./MessageComponent";

interface ChatMessagesPaneProps {
  messages: ChatMessage[];
  isLoading: boolean;
}

export default function ChatMessagesPane({ messages, isLoading }: ChatMessagesPaneProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);
  const [showScrollBtn, setShowScrollBtn] = useState(false);

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
    setShowScrollBtn(!atBottom);
  };

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) {
      userScrolledRef.current = false;
      setShowScrollBtn(false);
    }
  };

  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    userScrolledRef.current = false;
    setShowScrollBtn(false);
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
    <div className="relative flex-1 overflow-hidden">
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        onScroll={handleScroll}
        className="h-full space-y-3 overflow-y-auto py-4 sm:space-y-4"
      >
        {messages.map((message, index) => {
          const prevMessage = index > 0 ? messages[index - 1] : null;
          return (
            <div key={message.id} className="message-slide-in">
              <MessageComponent
                message={message}
                prevMessage={prevMessage}
              />
            </div>
          );
        })}
      </div>
      {showScrollBtn && (
        <button
          type="button"
          onClick={scrollToBottom}
          title="Scroll to bottom"
          className="absolute bottom-4 right-4 flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-md transition-all hover:bg-primary/90"
        >
          <ChevronDown className="h-4 w-4" />
        </button>
      )}
    </div>
  );
}

import React, { memo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { twMerge } from "tailwind-merge";
import type { ChatMessage } from "../types";
import MessageCopyControl from "./MessageCopyControl";
import ToolRenderer from "./ToolRenderer";

interface MessageComponentProps {
  message: ChatMessage;
  prevMessage: ChatMessage | null;
}

const MessageComponent = memo(({ message, prevMessage }: MessageComponentProps) => {
  const isGrouped =
    prevMessage !== null &&
    prevMessage.type === message.type &&
    !message.isToolUse &&
    !prevMessage.isToolUse;

  const formattedTime = new Date(message.timestamp).toLocaleTimeString();
  const [hovered, setHovered] = useState(false);

  if (message.type === "user") {
    return (
      <div
        className="flex justify-end px-3 py-0.5"
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <div className="max-w-xs sm:max-w-md lg:max-w-lg xl:max-w-xl">
          <div className="rounded-2xl rounded-br-md bg-blue-600 px-3 py-2 text-sm text-white shadow-sm">
            <div className="whitespace-pre-wrap break-words">{message.content}</div>
            <div className="mt-1 flex items-center justify-end gap-2">
              {hovered && (
                <MessageCopyControl content={message.content} messageType="user" />
              )}
              <span className="text-[10px] text-blue-100">{formattedTime}</span>
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (message.type === "error") {
    return (
      <div className="px-3 py-0.5">
        <div className="rounded-lg border border-red-500/30 bg-red-950/20 px-3 py-2 text-sm text-red-300">
          <span className="font-medium">Error: </span>{message.content}
        </div>
      </div>
    );
  }

  if (message.isThinking) {
    if (!message.content) {
      return (
        <div className="px-3 py-0.5">
          <ThinkingIndicator />
        </div>
      );
    }
    return (
      <div className="px-3 py-0.5">
        <details className="group">
          <summary className="flex cursor-pointer list-none items-center gap-2 text-xs font-medium text-muted-foreground hover:text-foreground">
            <svg
              className="h-3 w-3 transition-transform group-open:rotate-90"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
            Thinking…
          </summary>
          <div className="mt-2 border-l-2 border-muted pl-3 text-xs text-muted-foreground">
            <div className="whitespace-pre-wrap">{message.content}</div>
          </div>
        </details>
      </div>
    );
  }

  if (message.isToolUse) {
    return (
      <div className="px-3 py-0.5">
        <ToolRenderer
          toolName={message.toolName ?? ""}
          toolInput={message.toolInput ?? {}}
          toolResult={message.toolResult}
        />
      </div>
    );
  }

  // Regular assistant message
  return (
    <div
      className={twMerge("px-3", isGrouped ? "py-0.5" : "py-1")}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {!isGrouped && (
        <div className="mb-1.5 flex items-center gap-2">
          <div className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-primary text-[10px] font-bold text-primary-foreground">
            C
          </div>
          <span className="text-xs font-medium text-foreground">Claude</span>
          <span className="text-[10px] text-muted-foreground">{formattedTime}</span>
          <div className="ml-auto">
            {hovered && (
              <MessageCopyControl content={message.content} messageType="assistant" />
            )}
          </div>
        </div>
      )}
      <div className={twMerge("min-w-0 overflow-x-auto text-sm text-foreground", isGrouped ? "" : "pl-8")}>
        <MarkdownContent content={message.content} />
      </div>
      {isGrouped && hovered && (
        <div className="flex justify-end pt-0.5">
          <MessageCopyControl content={message.content} messageType="assistant" />
        </div>
      )}
    </div>
  );
});

MessageComponent.displayName = "MessageComponent";

export default MessageComponent;

function MarkdownContent({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      className="prose prose-sm max-w-none dark:prose-invert prose-pre:bg-gray-900 prose-pre:border prose-pre:border-border prose-code:text-sm"
    >
      {content}
    </ReactMarkdown>
  );
}

function ThinkingIndicator() {
  return (
    <div className="flex items-center gap-2 py-1">
      <div className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-primary text-[10px] font-bold text-primary-foreground">
        C
      </div>
      <div className="flex items-center gap-1">
        <span className="thinking-dot h-1.5 w-1.5 rounded-full bg-muted-foreground" />
        <span className="thinking-dot h-1.5 w-1.5 rounded-full bg-muted-foreground" />
        <span className="thinking-dot h-1.5 w-1.5 rounded-full bg-muted-foreground" />
      </div>
    </div>
  );
}

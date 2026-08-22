/**
 * ChatPage — a chat UI backed by the DeepSeek Harness, built with Vercel AI
 * Elements (shadcn/ui) + the Vercel AI SDK.
 *
 * - `useChat` (@ai-sdk/react v4) posts to /api/chat (Vite-proxied to the
 *   harness-chat-adapter), which translates into DeepSeek Harness RPC and
 *   streams a UI Message Stream (v1) back.
 * - Rendering uses the AI Elements `Conversation` / `Message` / `PromptInput`
 *   components; assistant text is markdown-rendered via Streamdown
 *   (`MessageResponse`).
 */
import { useChat } from "@ai-sdk/react";
import { CopyIcon, RefreshCcwIcon } from "lucide-react";
import { Fragment, useCallback } from "react";
import { writeClipboard } from "@/lib/clipboard";
import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from "@/components/ai-elements/conversation";
import {
  Message,
  MessageActions,
  MessageAction,
  MessageContent,
  MessageResponse,
} from "@/components/ai-elements/message";
import {
  PromptInput,
  PromptInputSubmit,
  PromptInputTextarea,
  type PromptInputMessage,
} from "@/components/ai-elements/prompt-input";

export function ChatPage() {
  const { messages, sendMessage, status, stop, regenerate } = useChat();

  const handleSubmit = useCallback(
    (message: PromptInputMessage) => {
      if (message.text.trim()) {
        sendMessage({ text: message.text });
      }
    },
    [sendMessage],
  );

  const isGenerating = status === "submitted" || status === "streaming";

  return (
    <div className="relative flex h-[calc(100vh-64px)] flex-col">
      <Conversation>
        <ConversationContent>
          {messages.length === 0 ? (
            <div className="mx-auto mt-16 flex max-w-md flex-col items-center gap-2 text-center">
              <h2 className="text-lg font-semibold">Harness Chat</h2>
              <p className="text-sm text-muted-foreground">
                Ask the DeepSeek Harness backend anything. Replies stream from{" "}
                <code className="rounded bg-muted px-1 py-0.5">deepseek-v4-flash</code>{" "}
                through the Vercel AI SDK.
              </p>
            </div>
          ) : (
            messages.map((message) => (
              <Fragment key={message.id}>
                {message.parts.map((part, partIndex) => {
                  switch (part.type) {
                    case "text": {
                      const isLastMessage =
                        message.id === messages[messages.length - 1]?.id;
                      const isLastTextPart =
                        isLastMessage &&
                        partIndex ===
                          message.parts.filter((p) => p.type !== "step-start").length - 1;
                      return (
                        <Message key={partIndex} from={message.role}>
                          <MessageContent>
                            <MessageResponse>{part.text}</MessageResponse>
                          </MessageContent>
                          {isLastTextPart && message.role === "assistant" && (
                            <MessageActions>
                              <MessageAction
                                tooltip="Copy"
                                label="Copy"
                                onClick={() => writeClipboard(part.text)}
                              >
                                <CopyIcon className="size-4" />
                              </MessageAction>
                              <MessageAction
                                tooltip="Regenerate"
                                label="Regenerate"
                                disabled={isGenerating}
                                onClick={() => regenerate({ messageId: message.id })}
                              >
                                <RefreshCcwIcon className="size-4" />
                              </MessageAction>
                            </MessageActions>
                          )}
                        </Message>
                      );
                    }
                    default:
                      return null;
                  }
                })}
              </Fragment>
            ))
          )}
        </ConversationContent>
        <ConversationScrollButton />
      </Conversation>

      <div className="border-t bg-background/95 p-3 backdrop-blur">
        <PromptInput onSubmit={handleSubmit}>
          <PromptInputTextarea disabled={isGenerating} />
          <PromptInputSubmit status={status} onStop={stop} />
        </PromptInput>
      </div>
    </div>
  );
}

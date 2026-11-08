import type { Message, ToolCall, ToolResultMessage } from "@earendil-works/pi-ai";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import type { Logger } from "./logger.ts";
import { isRecord } from "./util.ts";

export const HARNESS_NAME = "neo";
export const INTERRUPTED_RESULT = "interrupted: the daemon restarted";

function isMessage(value: unknown): value is Message {
  return (
    isRecord(value) &&
    (value.role === "user" || value.role === "assistant" || value.role === "toolResult")
  );
}

function interruptedResult(call: ToolCall, timestamp: number): ToolResultMessage {
  return {
    role: "toolResult",
    toolCallId: call.id,
    toolName: call.name,
    content: [{ type: "text", text: INTERRUPTED_RESULT }],
    isError: true,
    timestamp,
  };
}

/**
 * Makes a message list valid to send after a crash mid-turn:
 * - Failed or aborted assistant messages are dropped. pi-ai skips them on every request
 *   anyway, so dropping them changes nothing the model sees.
 * - Every tool call gets exactly one result directly after its assistant message; missing ones
 *   are synthesized as errors and stray results are dropped.
 * The output depends only on the input (timestamps come from the messages), so the same history
 * always yields the same context and provider prompt caches stay valid.
 */
export function repairMessages(messages: Message[]): Message[] {
  const repaired: Message[] = [];
  for (let i = 0; i < messages.length; i++) {
    const message = messages[i];
    if (message?.role === "user") {
      repaired.push(message);
    } else if (message?.role === "assistant") {
      let end = i + 1;
      const results = new Map<string, ToolResultMessage>();
      for (let next = messages[end]; next?.role === "toolResult"; next = messages[++end]) {
        if (!results.has(next.toolCallId)) results.set(next.toolCallId, next);
      }
      i = end - 1;
      if (message.stopReason === "error" || message.stopReason === "aborted") continue;
      repaired.push(message);
      for (const block of message.content) {
        if (block.type !== "toolCall") continue;
        repaired.push(results.get(block.id) ?? interruptedResult(block, message.timestamp));
      }
    }
    // A tool result that isn't directly behind its assistant message has no valid place.
  }
  return repaired;
}

/** Our own messages from the session log, as logged. Everything else is ignored. */
export function loggedMessages(history: SessionEvent[], logger?: Logger): Message[] {
  const messages: Message[] = [];
  for (const event of history) {
    if (event.type !== "harness_message" || event.harness !== HARNESS_NAME) continue;
    if (isMessage(event.message)) messages.push(event.message);
    else logger?.warn("Ignoring a harness_message that is not a neo message.");
  }
  return messages;
}

/**
 * Rebuilds the neo context from the session log, from its last context_cleared (/clear) on.
 * Everything but our own messages is ignored.
 */
export function restoreMessages(history: SessionEvent[], logger?: Logger): Message[] {
  const cleared = history.findLastIndex((event) => event.type === "context_cleared");
  return repairMessages(loggedMessages(history.slice(cleared + 1), logger));
}

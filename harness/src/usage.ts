import type { AssistantMessage, Message } from "@mariozechner/pi-ai";
import type { Usage } from "@neoworks-dev/neo-protocol/types";

/** Tokens a request used: everything it sent (cached or not) plus what it generated. */
function tokensOf({ usage }: AssistantMessage): number {
  return usage.input + usage.cacheRead + usage.cacheWrite + usage.output;
}

/**
 * Running totals for one session. Seeded from the messages already in the log, so cost and token
 * counts keep adding up across daemon restarts.
 */
export interface UsageTracker {
  /** Adds an assistant message; the event to log, or undefined when it reported no usage. */
  add(message: AssistantMessage): Usage | undefined;
}

export function createUsageTracker(
  previous: Message[],
  /** The context window of the session's current model. */
  contextSize: () => number,
): UsageTracker {
  let cost = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  const count = (message: AssistantMessage): void => {
    const { usage } = message;
    cost += usage.cost.total;
    inputTokens += usage.input + usage.cacheRead + usage.cacheWrite;
    outputTokens += usage.output;
  };
  for (const message of previous) if (message.role === "assistant") count(message);

  return {
    add(message) {
      const contextUsed = tokensOf(message);
      // Failed requests report nothing; they must not reset the numbers shown.
      if (contextUsed === 0) return undefined;
      count(message);
      return {
        type: "usage",
        contextUsed,
        contextSize: contextSize(),
        cost,
        inputTokens,
        outputTokens,
      };
    },
  };
}

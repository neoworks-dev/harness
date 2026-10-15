// A scripted stand-in for the real harness, selected with SUITE_AGENT=echo. Deterministic and
// offline, so end-to-end tests can drive the daemon and CLI without a model.
//
// Each turn replies "echo #<n>: <text>" in a few chunks, where n counts turns including those
// restored from history. "[tool]" in the text adds a tool call; "wait:<ms>" delays the reply
// (cancellable).
import type { Agent, AgentSession, PromptParams, PromptResult } from "@suite/harness/agent";
import type { SessionEvent } from "@suite/protocol/types";

function sleep(ms: number, signal: AbortSignal): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve(false);
    const timer = setTimeout(() => resolve(true), ms);
    signal.addEventListener("abort", () => {
      clearTimeout(timer);
      resolve(false);
    });
  });
}

class EchoSession implements AgentSession {
  private controller?: AbortController;

  constructor(private turns: number) {}

  async prompt({ text, pendingEvents, onUpdate }: PromptParams): Promise<PromptResult> {
    const controller = new AbortController();
    this.controller = controller;
    this.turns += 1;
    const messageId = `echo-${this.turns}`;
    const prefix = pendingEvents.map((e) => `[${e.type}] `).join("");
    onUpdate({ type: "harness_message", harness: "echo", message: { role: "user", text } });

    const wait = /wait:(\d+)/.exec(text);
    if (wait && !(await sleep(Number(wait[1]), controller.signal))) {
      return { stopReason: "cancelled" };
    }
    if (text.includes("[tool]")) {
      const toolCallId = `${messageId}-tool`;
      onUpdate({
        type: "tool_call",
        toolCallId,
        title: "echo tool",
        kind: "other",
        status: "pending",
      });
      onUpdate({
        type: "tool_call_update",
        toolCallId,
        status: "completed",
        content: [{ type: "text", text: "tool output" }],
      });
    }
    const reply = `${prefix}echo #${this.turns}: ${text}`;
    for (const piece of reply.match(/.{1,8}/gs) ?? []) {
      if (!(await sleep(2, controller.signal))) return { stopReason: "cancelled" };
      onUpdate({ type: "agent_message_chunk", messageId, text: piece });
    }
    onUpdate({ type: "harness_message", harness: "echo", message: { role: "assistant", reply } });
    return { stopReason: "end_turn" };
  }

  cancel(): void {
    this.controller?.abort();
  }
}

function countTurns(history: SessionEvent[]): number {
  return history.filter(
    (e) => e.type === "harness_message" && e.harness === "echo" && hasRole(e.message, "user"),
  ).length;
}

function hasRole(message: unknown, role: string): boolean {
  return (
    typeof message === "object" && message !== null && "role" in message && message.role === role
  );
}

export function createEchoAgent(): Agent {
  return {
    name: "echo",
    newSession: () => Promise.resolve(new EchoSession(0)),
    loadSession: ({ history }) => Promise.resolve(new EchoSession(countTurns(history))),
  };
}

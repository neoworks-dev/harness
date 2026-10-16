// The main agent's delegate tool. Starting a guest is the user's decision: each delegation asks
// first (start it, allow it for the rest of the session, use another guest instead, or reject),
// and the guest runs with the settings the user chose for it.
import type { AgentUpdate } from "@suite/harness/agent";
import type { DelegateArgs } from "@suite/protocol/core-tools";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { PermissionOption, SessionEvent } from "@suite/protocol/types";
import type { Asker } from "../asker.ts";
import type { HarnessSettings } from "../settings.ts";
import type { GuestManager } from "./guests.ts";

const TASK_PREVIEW = 120;

export interface DelegateParts {
  guests: GuestManager;
  asker: Asker;
  settings: (session: string, guest: string) => HarnessSettings;
  append: (session: string, event: SessionEvent) => void;
  /** Who asks: the session agent's harness name. */
  main: string;
  /** Whether the user lets this session start guests without asking (/approvals auto). */
  autoApprove?: (session: string) => boolean;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/** "model sonnet, effort high", or "" when the user chose nothing. */
function describeSettings(settings: HarnessSettings): string {
  const names: Record<string, string> = { model: "model", thought_level: "effort", mode: "mode" };
  return Object.entries(settings)
    .map(([category, value]) => `${names[category] ?? category} ${value}`)
    .join(", ");
}

export function createDelegate(
  parts: DelegateParts,
): (session: string, args: DelegateArgs, signal?: AbortSignal) => Promise<CallToolResult> {
  const { guests, asker } = parts;
  /** session\0guest pairs the user allowed for the rest of the session. */
  const allowed = new Set<string>();

  async function decide(
    session: string,
    agent: string,
    task: string,
    signal?: AbortSignal,
  ): Promise<string | CallToolResult> {
    if (allowed.has(`${session}\0${agent}`) || parts.autoApprove?.(session) === true) return agent;
    const title = guests.title(agent);
    const options: PermissionOption[] = [
      { id: "allow", name: `Start ${title}`, kind: "allow_once" },
      { id: "always", name: `Allow ${title} for this session`, kind: "allow_always" },
    ];
    for (const other of guests.names().filter((n) => n !== agent)) {
      options.push({ id: `use:${other}`, name: `Use ${guests.title(other)}`, kind: "allow_once" });
    }
    options.push({ id: "reject", name: "Reject", kind: "reject_once" });
    const settings = describeSettings(parts.settings(session, agent));
    const preview = task.length > TASK_PREVIEW ? `${task.slice(0, TASK_PREVIEW)}…` : task;
    let question = `Hand a task to ${title}`;
    if (settings !== "") question += ` (${settings})`;
    const answer = await asker.ask(
      session,
      {
        type: "permission_request",
        agent: parts.main,
        title: `${question}: ${preview}`,
        options,
      },
      signal,
    );
    if (answer === undefined) return text("No answer: the turn ended first.", true);
    if (answer === "reject") {
      return text(
        `The user declined to hand this to ${title}. Do it yourself, or ask them how to proceed.`,
        true,
      );
    }
    if (answer === "always") allowed.add(`${session}\0${agent}`);
    return answer.startsWith("use:") ? answer.slice(4) : agent;
  }

  return async (session, { agent, task }, signal) => {
    if (!guests.has(agent)) {
      const names = guests.names();
      const known = names.length > 0 ? names.join(", ") : "none are configured";
      return text(`No guest named ${agent} (${known}).`, true);
    }
    const decision = await decide(session, agent, task, signal);
    if (typeof decision !== "string") return decision;
    const emit = (update: AgentUpdate): void => {
      parts.append(session, update);
    };
    const result = await guests.run(session, decision, task, { emit, signal });
    const lines: string[] = [];
    if (decision !== agent) lines.push(`(The user chose ${guests.title(decision)} instead.)`);
    lines.push(result.text || "(no reply)");
    if (result.stopReason !== "end_turn") {
      lines.push(`[${decision} stopped: ${result.error ?? result.stopReason}]`);
    }
    return text(
      lines.join("\n\n"),
      result.stopReason === "error" || result.stopReason === "cancelled",
    );
  };
}

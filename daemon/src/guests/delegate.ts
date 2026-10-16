// The delegate tool of the session's agent (whatever harness it runs on). Starting a subagent is
// the user's decision: each delegation asks first (start it, allow it for the rest of the
// session, use another harness instead, or reject), naming the harness, model and effort that
// will run. Without model and effort in the call the subagent runs with the settings the user
// chose for it, else its own defaults.
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
  /** Who asks: the harness the session runs on. */
  main: (session: string) => string;
  /** Whether the user lets this session start guests without asking (/approvals auto). */
  autoApprove?: (session: string) => boolean;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/** "model sonnet, effort high", or "" when nothing is chosen. */
function describeSettings(settings: HarnessSettings): string {
  const parts: string[] = [];
  if (settings.model !== undefined) parts.push(`model ${settings.model}`);
  if (settings.thought_level !== undefined) parts.push(`effort ${settings.thought_level}`);
  if (settings.mode !== undefined) parts.push(`mode ${settings.mode}`);
  return parts.join(", ");
}

/** Which harness runs and with which settings of its own for this task. */
interface Decision {
  agent: string;
  settings: HarnessSettings;
}

/** What the call asks for itself: its model and effort. */
function explicitSettings(args: DelegateArgs): HarnessSettings {
  const settings: HarnessSettings = {};
  if (args.model !== undefined && args.model !== "") settings.model = args.model;
  if (args.effort !== undefined && args.effort !== "") settings.thought_level = args.effort;
  return settings;
}

export function createDelegate(
  parts: DelegateParts,
): (session: string, args: DelegateArgs, signal?: AbortSignal) => Promise<CallToolResult> {
  const { guests, asker } = parts;
  /** session, harness and settings the user allowed for the rest of the session. */
  const allowed = new Set<string>();

  /** The harness to run (the user may pick another) and the settings to run it with. */
  async function decide(
    session: string,
    agent: string,
    task: string,
    explicit: HarnessSettings,
    signal?: AbortSignal,
  ): Promise<Decision | { refusal: CallToolResult }> {
    // What will run: the call's own choices win over the user's.
    const effective = { ...parts.settings(session, agent), ...explicit };
    const key = `${session}\0${agent}\0${JSON.stringify(effective)}`;
    if (allowed.has(key) || parts.autoApprove?.(session) === true) {
      return { agent, settings: explicit };
    }
    const title = guests.title(agent);
    const settings = describeSettings(effective);
    const options: PermissionOption[] = [
      { id: "allow", name: `Start ${title}`, kind: "allow_once" },
      { id: "always", name: `Allow ${title} for this session`, kind: "allow_always" },
    ];
    for (const other of guests.harnesses().filter((n) => n !== agent)) {
      options.push({ id: `use:${other}`, name: `Use ${guests.title(other)}`, kind: "allow_once" });
    }
    options.push({ id: "reject", name: "Reject", kind: "reject_once" });
    const preview = task.length > TASK_PREVIEW ? `${task.slice(0, TASK_PREVIEW)}…` : task;
    let question = `Hand a task to ${title}`;
    if (settings !== "") question += ` (${settings})`;
    const answer = await asker.ask(
      session,
      {
        type: "permission_request",
        agent: parts.main(session),
        title: `${question}: ${preview}`,
        options,
      },
      signal,
    );
    if (answer === undefined) return { refusal: text("No answer: the turn ended first.", true) };
    if (answer === "reject") {
      const declined = `The user declined to hand this to ${title}. Do it yourself, or ask them how to proceed.`;
      return { refusal: text(declined, true) };
    }
    if (answer === "always") allowed.add(key);
    // Another harness knows other model names: the call's model and effort don't carry over.
    if (answer.startsWith("use:")) return { agent: answer.slice(4), settings: {} };
    return { agent, settings: explicit };
  }

  return async (session, args, signal) => {
    const { agent, task } = args;
    const harnesses = guests.harnesses();
    if (!harnesses.includes(agent)) {
      const known = harnesses.length > 0 ? harnesses.join(", ") : "none are configured";
      return text(`No guest named ${agent} (${known}).`, true);
    }
    const decision = await decide(session, agent, task, explicitSettings(args), signal);
    if ("refusal" in decision) return decision.refusal;
    const emit = (update: AgentUpdate): void => {
      parts.append(session, update);
    };
    const run = { emit, signal, settings: decision.settings };
    const result = await guests.run(session, decision.agent, task, run);
    const lines: string[] = [];
    if (decision.agent !== agent) {
      lines.push(`(The user chose ${guests.title(decision.agent)} instead.)`);
    }
    lines.push(result.text || "(no reply)");
    if (result.stopReason !== "end_turn") {
      lines.push(`[${decision.agent} stopped: ${result.error ?? result.stopReason}]`);
    }
    return text(
      lines.join("\n\n"),
      result.stopReason === "error" || result.stopReason === "cancelled",
    );
  };
}

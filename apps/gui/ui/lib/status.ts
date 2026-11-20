// What the status line shows about a session, folded from its events: whether a turn runs, the
// harness and its settings, the approvals mode and the agent's usage. Plain functions, so they
// are tested without a window.
import type {
  ConfigCategory,
  HarnessSettings as Offered,
  SessionEventRecord,
  Usage,
} from "@neoworks-dev/neo-protocol/types";

export type Approvals = "ask" | "review" | "edits" | "auto";

/** Shift-Tab goes through them in this order. */
export const APPROVALS: readonly Approvals[] = ["ask", "review", "edits", "auto"];

export function nextApprovals(mode: Approvals): Approvals {
  const next = APPROVALS[(APPROVALS.indexOf(mode) + 1) % APPROVALS.length];
  return next ?? "ask";
}

function isApprovals(value: string): value is Approvals {
  return APPROVALS.some((mode) => mode === value);
}

/** One harness's model choices; absent means the harness's own default. */
export interface HarnessSettings {
  model?: string;
  effort?: string;
  mode?: string;
}

export interface SessionStatus {
  /** From a user_message until its turn_ended. */
  running: boolean;
  /** When the running turn started (ms), for the elapsed time. */
  turnStartedAt?: number;
  /** The harness the session runs on: neo, claude, codex… */
  harness: string;
  approvals: Approvals;
  /** Per harness, so switching back shows that harness's choices again. */
  settings: Record<string, HarnessSettings>;
  /** The session's own agent; guests and subagents report their own. */
  usage?: Usage;
  /** When the last turn ended (ms): the provider's prompt cache counts from there. */
  lastTurnAt?: number;
  /** The context was cleared (/clear) since the last turn: there is no cache to go cold. */
  cleared?: boolean;
}

/** Idle this long, a session's prompt cache is gone with Anthropic (5 minutes, an hour with long
 * retention) and OpenAI (5 minutes to an hour), as the terminal app counts it. */
export const CACHE_COLD_MS = 60 * 60 * 1000;

/** Whether the next turn starts without the provider's prompt cache: the last ended over an hour ago. */
export function cacheCold(status: SessionStatus, now: number): boolean {
  return (
    !status.running &&
    !status.cleared &&
    status.lastTurnAt !== undefined &&
    now - status.lastTurnAt > CACHE_COLD_MS
  );
}

export function initialStatus(): SessionStatus {
  return { running: false, harness: "neo", approvals: "ask", settings: {} };
}

/** The status after one more event. Returns the same object when the event changes nothing. */
export function applyEvent(status: SessionStatus, record: SessionEventRecord): SessionStatus {
  const { event } = record;
  switch (event.type) {
    case "user_message":
      return { ...status, running: true, turnStartedAt: record.ts, cleared: false };
    case "turn_ended":
      return { ...status, running: false, turnStartedAt: undefined, lastTurnAt: record.ts };
    case "context_cleared":
      return { ...status, cleared: true };
    case "usage":
      if (event.agent !== undefined) return status;
      return { ...status, usage: event };
    case "setting_changed": {
      const { harness, option, value } = event;
      if (harness === "daemon") {
        if (option === "harness") return { ...status, harness: value };
        if (option === "approvals" && isApprovals(value)) return { ...status, approvals: value };
        return status;
      }
      if (option !== "model" && option !== "effort" && option !== "mode") return status;
      const current = status.settings[harness] ?? {};
      return {
        ...status,
        settings: { ...status.settings, [harness]: { ...current, [option]: value } },
      };
    }
    default:
      return status;
  }
}

/** The settings of the harness the session runs on now. */
export function currentSettings(status: SessionStatus): HarnessSettings {
  return status.settings[status.harness] ?? {};
}

/**
 * The name a harness gives a value of one of its settings (`Haiku 5.5` for the model `haiku`),
 * from what it offers; undefined when it doesn't list the value.
 */
export function optionName(
  offered: readonly Offered[],
  harness: string,
  category: ConfigCategory,
  value: string,
): string | undefined {
  const option = offered
    .find((entry) => entry.name === harness)
    ?.options.find((o) => o.category === category);
  return option?.values.find((entry) => entry.value === value)?.name;
}

/** What to show for a setting's value: the harness's name for it, else the value itself. */
export function optionLabel(
  offered: readonly Offered[],
  harness: string,
  category: ConfigCategory,
  value: string,
): string {
  return optionName(offered, harness, category, value) ?? value;
}

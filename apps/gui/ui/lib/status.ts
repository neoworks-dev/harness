// What the status line shows about a session, folded from its events: whether a turn runs, the
// harness and its settings, the approvals mode and the agent's usage. Plain functions, so they
// are tested without a window.
import type { SessionEventRecord, Usage } from "@suite/protocol/types";

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
  /** The harness the session runs on: pi, claude, codex… */
  harness: string;
  approvals: Approvals;
  /** Per harness, so switching back shows that harness's choices again. */
  settings: Record<string, HarnessSettings>;
  /** The session's own agent; guests and subagents report their own. */
  usage?: Usage;
}

export function initialStatus(): SessionStatus {
  return { running: false, harness: "pi", approvals: "ask", settings: {} };
}

/** The status after one more event. Returns the same object when the event changes nothing. */
export function applyEvent(status: SessionStatus, record: SessionEventRecord): SessionStatus {
  const { event } = record;
  switch (event.type) {
    case "user_message":
      return { ...status, running: true, turnStartedAt: record.ts };
    case "turn_ended":
      return { ...status, running: false, turnStartedAt: undefined };
    case "usage":
      if (event.agent !== undefined) return status;
      return { ...status, usage: event };
    case "setting_changed": {
      const { harness, option, value } = event;
      if (harness === "suite") {
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

// The text of the status line under the input, as segments with a meaning (the window maps the
// tone to a colour). Plain functions, so they are tested without a window.
import type { ConnectionState } from "@neoworks-dev/neo-sdk";
import type { Usage } from "@neoworks-dev/neo-protocol/types";
import { cacheCold, currentSettings, type Approvals, type SessionStatus } from "../../lib/status.ts";

export type Tone = "fg" | "muted" | "accent" | "danger" | "success";

export interface Segment {
  text: string;
  tone: Tone;
  /** Drawn before the text. */
  icon?: "branch";
}

/** The text with its first letter a capital: "Working 7s", "Plan". */
export function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** ask = muted, review and edits = accent, auto = danger (nothing asks first). */
export function approvalsTone(mode: Approvals): Tone {
  if (mode === "auto") return "danger";
  if (mode === "ask") return "muted";
  return "accent";
}

/** "7s", "2m 05s", "1h 03m". */
export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/** How full the context window is, in whole percent; undefined when either number is unknown. */
export function contextPercent(usage: Usage | undefined): number | undefined {
  const used = usage?.contextUsed;
  const size = usage?.contextSize;
  if (used === undefined || size === undefined || size <= 0) return undefined;
  return Math.round((used / size) * 100);
}

/** "$0.04", "$1.20"; a tenth of a cent shows as "<$0.01". */
export function formatCost(cost: number | undefined): string | undefined {
  if (cost === undefined) return undefined;
  if (cost > 0 && cost < 0.01) return "<$0.01";
  return `$${cost.toFixed(2)}`;
}

export interface StatusInput {
  branch: string | null;
  status: SessionStatus;
  queued: number;
  connection: ConnectionState | "starting";
  now: number;
}

/**
 * The status line's runs, left to right, each followed by a separator when more come. The harness,
 * model, reasoning level and approvals are the input's pickers, so they are not repeated here.
 */
export function statusSegments(input: StatusInput): Segment[] {
  const { status } = input;
  const settings = currentSettings(status);
  const segments: Segment[] = [];
  const add = (text: string | null | undefined, tone: Tone = "muted"): void => {
    if (text !== null && text !== undefined && text !== "") {
      segments.push({ text: capitalized(text), tone });
    }
  };

  if (input.branch) segments.push({ text: input.branch, tone: "muted", icon: "branch" });
  add(settings.mode);

  if (status.running) {
    let text = "Working";
    if (status.turnStartedAt !== undefined) {
      text += ` ${formatElapsed(input.now - status.turnStartedAt)}`;
    }
    add(text, "accent");
  }
  if (input.queued > 0) add(`Queued ${input.queued}`, "accent");
  const percent = contextPercent(status.usage);
  if (percent !== undefined) add(`Context ${percent}%`, percent >= 90 ? "danger" : "muted");
  add(formatCost(status.usage?.cost));
  if (cacheCold(status, input.now)) add("Cache expired", "accent");
  if (input.connection !== "open" && input.connection !== "starting")
    add(input.connection, "danger");
  return segments;
}

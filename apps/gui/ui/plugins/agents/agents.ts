// The agents under the prompt, folded from the session's events: `subagent_started` and
// `subagent_ended` mark a delegation, and events labelled with the agent's harness name say what
// it does (apps/term/src/agents.rs does the same). Plain functions on immutable rows.
import type { SessionEventRecord, SessionInfo, StopReason } from "@neoworks-dev/neo-protocol/types";

/** How long a finished agent stays in the list. */
export const LINGER_MS = 5000;
/** Rows shown before the rest is folded into "+N more". */
export const MAX_ROWS = 6;

export type AgentState = "running" | "done" | "failed" | "stopped";

export interface AgentRow {
  id: string;
  /** Harness name: neo, claude, codex… */
  harness: string;
  /** The agent definition it runs as, shown instead of the harness. */
  name?: string;
  model?: string;
  task: string;
  /** The directory it works in, when that isn't the session's. */
  cwd?: string;
  /** The session it runs as, when it works in a directory of its own. */
  child?: string;
  /** Started with `background`: it keeps running when the turn that started it ends. */
  background?: boolean;
  state: AgentState;
  startedMs: number;
  endedMs?: number;
  /** Its latest tool call, or what it does between them. */
  activity: string;
  tokens?: number;
}

function tokensOf(usage: {
  inputTokens?: number;
  outputTokens?: number;
  contextUsed?: number;
}): number | undefined {
  if (usage.inputTokens === undefined && usage.outputTokens === undefined) return usage.contextUsed;
  return (usage.inputTokens ?? 0) + (usage.outputTokens ?? 0);
}

/** What a labelled event says about the running agent of that name, which must not be a session of its own. */
function note(
  rows: readonly AgentRow[],
  label: string | undefined,
  change: Partial<AgentRow>,
): readonly AgentRow[] {
  if (label === undefined) return rows;
  let at = -1;
  rows.forEach((row, index) => {
    if (row.harness === label && row.state === "running" && row.child === undefined) at = index;
  });
  const row = rows[at];
  if (!row) return rows;
  return rows.map((r) => (r === row ? { ...r, ...change } : r));
}

function endState(stopReason: StopReason): AgentState {
  if (stopReason === "end_turn") return "done";
  if (stopReason === "cancelled") return "stopped";
  return "failed";
}

/** The rows after one more event of the shown session. Returns the same array when nothing changes. */
export function applyAgentEvent(
  rows: readonly AgentRow[],
  record: SessionEventRecord,
): readonly AgentRow[] {
  const { event } = record;
  switch (event.type) {
    case "subagent_started": {
      if (rows.some((row) => row.id === event.id)) return rows;
      const row: AgentRow = {
        id: event.id,
        harness: event.agent,
        task: event.task,
        state: "running",
        startedMs: record.ts,
        activity: "starting…",
      };
      if (event.name !== undefined) row.name = event.name;
      if (event.model !== undefined) row.model = event.model;
      if (event.cwd !== undefined) row.cwd = event.cwd;
      if (event.child !== undefined) row.child = event.child;
      if (event.background === true) row.background = true;
      return [...rows, row];
    }
    case "subagent_ended": {
      const state = endState(event.stopReason);
      const activity = state === "failed" ? (event.error ?? "failed") : state;
      return rows.map((row) =>
        row.id === event.id ? { ...row, state, endedMs: record.ts, activity } : row,
      );
    }
    // A delegation lives inside a turn: when the turn is over nothing still runs, but for the ones
    // that were started in the background.
    case "turn_ended":
      if (!rows.some((row) => row.state === "running" && row.background !== true)) return rows;
      return rows.map((row) =>
        row.state === "running" && row.background !== true
          ? { ...row, state: "failed", endedMs: record.ts, activity: "interrupted" }
          : row,
      );
    // A message to or from a running subagent is what it does now.
    case "agent_message":
      return rows.map((row) => {
        if (row.state !== "running") return row;
        if (row.id === event.to) return { ...row, activity: `← ${event.from}` };
        if (row.id === event.from) return { ...row, activity: `→ ${event.to}` };
        return row;
      });
    case "tool_call":
      return note(rows, event.agent, { activity: event.title });
    case "agent_thought_chunk":
    case "agent_message_chunk":
      return note(rows, event.agent, { activity: "thinking…" });
    case "usage": {
      const tokens = tokensOf(event);
      return tokens === undefined ? rows : note(rows, event.agent, { tokens });
    }
    default:
      return rows;
  }
}

/** Rows to draw: running ones and those that ended less than `LINGER_MS` ago. */
export function shownAgents(rows: readonly AgentRow[], nowMs: number): AgentRow[] {
  return rows.filter((row) => row.endedMs === undefined || nowMs - row.endedMs < LINGER_MS);
}

/**
 * What an agent that runs as a session of its own is doing, from that session: its latest
 * words, else what its row says. Events of other sessions don't reach the window.
 */
export function activityOf(row: AgentRow, sessions: readonly SessionInfo[]): string {
  if (row.child === undefined || row.state !== "running") return row.activity;
  const preview = sessions.find((s) => s.id === row.child)?.preview;
  const line = preview?.split(/\s+/).filter(Boolean).join(" ");
  return line ? line : row.activity;
}

/** `provider/model-id` reads as the id. */
export function modelLabel(model: string): string {
  return model.slice(model.lastIndexOf("/") + 1);
}

/** `5s`, `1m 12s`, `2h 03m`. */
export function elapsed(ms: number): string {
  const secs = Math.max(0, Math.floor(ms / 1000));
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${String(secs % 60).padStart(2, "0")}s`;
  return `${Math.floor(secs / 3600)}h ${String(Math.floor((secs % 3600) / 60)).padStart(2, "0")}m`;
}

/** `812`, `3.4k`, `1.2M`. */
export function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** The last path segment, for `⎇ dir`. */
export function dirLabel(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1);
}

/** The row to start on when the list gets the keyboard: the latest running one, else the last. */
export function firstSelection(shown: readonly AgentRow[]): string | undefined {
  const running = [...shown].reverse().find((row) => row.state === "running");
  return (running ?? shown[shown.length - 1])?.id;
}

/** The id `delta` rows from `from` among `shown`, stopping at the ends; a gone row starts at the first. */
export function moveAgent(
  shown: readonly AgentRow[],
  from: string | undefined,
  delta: number,
): string | undefined {
  if (shown.length === 0) return undefined;
  const at = Math.max(
    0,
    shown.findIndex((row) => row.id === from),
  );
  return shown[Math.min(shown.length - 1, Math.max(0, at + delta))]?.id;
}

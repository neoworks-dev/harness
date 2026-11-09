// What the Agents widget shows, from the daemon's session list: a row per top-level session worth
// a look (working, asking, or finished and unseen first, then the most recent), and today's totals.
// A session delegated into another counts toward its parent's row instead of getting its own.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";

export const MAX_ROWS = 8;

export type AgentStatus = "asking" | "working" | "waiting" | "unseen" | "idle";

export interface AgentRow {
  id: string;
  title: string;
  harness: string;
  status: AgentStatus;
  preview: string;
  /** USD, the session's and its subagents'. */
  cost: number;
  /** 0–1 of the context window of the agent that reported last; null when unknown. */
  contextFill: number | null;
  /** Subagent sessions running for it now. */
  subagents: number;
  lastActivity: number;
}

export interface UsageTotals {
  /** Whether the shell widget is connected to the daemon at all. */
  connected: boolean;
  /** USD over the sessions active since local midnight (each counted whole). */
  costToday: number;
  sessionsToday: number;
  working: number;
  asking: number;
}

export interface Summary {
  agents: AgentRow[];
  usage: UsageTotals;
}

const STATUS_ORDER: AgentStatus[] = ["asking", "working", "waiting", "unseen", "idle"];

export function summarize(sessions: SessionInfo[], now: Date, connected: boolean): Summary {
  const children = childrenByParent(sessions);
  const topLevel = sessions.filter((session) => session.parent === undefined);
  const rows = topLevel.map((session) => rowOf(session, children.get(session.id) ?? []));
  rows.sort(byInterest);
  return {
    agents: rows.slice(0, MAX_ROWS),
    usage: totalsOf(sessions, rows, startOfDay(now), connected),
  };
}

export function statusOf(session: SessionInfo): AgentStatus {
  if ((session.asking ?? 0) > 0) return "asking";
  if (session.state === "running") return session.waitingOn === undefined ? "working" : "waiting";
  if (session.unseen === true) return "unseen";
  return "idle";
}

function rowOf(session: SessionInfo, children: SessionInfo[]): AgentRow {
  const own = statusOf(session);
  // A question in a subagent is the user's to answer too.
  const status = children.some((child) => statusOf(child) === "asking") ? "asking" : own;
  return {
    id: session.id,
    title: session.title ?? "Untitled",
    harness: session.harness ?? "",
    status,
    preview: session.preview ?? "",
    cost: costOf(session) + children.reduce((sum, child) => sum + costOf(child), 0),
    contextFill: contextFillOf(session),
    subagents: children.filter((child) => child.state === "running").length,
    lastActivity: session.lastActivity,
  };
}

function childrenByParent(sessions: SessionInfo[]): Map<string, SessionInfo[]> {
  const children = new Map<string, SessionInfo[]>();
  for (const session of sessions) {
    if (session.parent === undefined) continue;
    const siblings = children.get(session.parent) ?? [];
    siblings.push(session);
    children.set(session.parent, siblings);
  }
  return children;
}

function byInterest(first: AgentRow, second: AgentRow): number {
  const order = STATUS_ORDER.indexOf(first.status) - STATUS_ORDER.indexOf(second.status);
  if (order !== 0) return order;
  return second.lastActivity - first.lastActivity;
}

function totalsOf(
  sessions: SessionInfo[],
  rows: AgentRow[],
  dayStart: number,
  connected: boolean,
): UsageTotals {
  const today = sessions.filter((session) => session.lastActivity >= dayStart);
  return {
    connected,
    costToday: today.reduce((sum, session) => sum + costOf(session), 0),
    sessionsToday: today.filter((session) => session.parent === undefined).length,
    working: rows.filter((row) => row.status === "working" || row.status === "waiting").length,
    asking: rows.filter((row) => row.status === "asking").length,
  };
}

function costOf(session: SessionInfo): number {
  return session.usage?.cost ?? 0;
}

function contextFillOf(session: SessionInfo): number | null {
  const used = session.usage?.contextUsed;
  const size = session.usage?.contextSize;
  if (used === undefined || size === undefined || size === 0) return null;
  return Math.min(1, used / size);
}

function startOfDay(now: Date): number {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

// How the widget words and colours what summary.ts computed. Kept out of the component so it is
// tested like the rest.
import type { AgentRow, AgentStatus, UsageTotals } from "../src/summary.ts";

export const ROWS_BY_SIZE: Record<string, number> = { small: 3, medium: 4, large: 8 };

/** The rows the source sent; anything that isn't a list shows as none. */
export function rowsOf(data: unknown): AgentRow[] {
  if (!Array.isArray(data)) return [];
  return data as AgentRow[];
}

const TONES: Record<AgentStatus, string> = {
  asking: "tone-warning",
  working: "tone-accent",
  waiting: "tone-accent",
  unseen: "tone-success",
  idle: "tone-neutral",
};

const LABELS: Record<AgentStatus, string> = {
  asking: "needs you",
  working: "working",
  waiting: "subagents working",
  unseen: "done",
  idle: "idle",
};

export function toneOf(status: AgentStatus): string {
  return `tone-dot ${TONES[status]}`;
}

export function statusLabel(row: AgentRow): string {
  if (row.subagents > 0 && row.status !== "asking")
    return `${LABELS[row.status]} · ${row.subagents} subagents`;
  return LABELS[row.status];
}

export function headline(usage: UsageTotals): string {
  if (!usage.connected) return "neo isn't running";
  const parts: string[] = [];
  if (usage.asking > 0) parts.push(`${usage.asking} need you`);
  if (usage.working > 0) parts.push(`${usage.working} working`);
  return parts.length === 0 ? "All quiet" : parts.join(" · ");
}

export function costLabel(cost: number): string {
  if (cost === 0) return "$0";
  if (cost < 0.01) return "<$0.01";
  return `$${cost.toFixed(2)}`;
}

export function todayLabel(usage: UsageTotals): string {
  const sessions = usage.sessionsToday === 1 ? "1 session" : `${usage.sessionsToday} sessions`;
  return `${costLabel(usage.costToday)} today · ${sessions}`;
}

/** The context meter's class: warning past 70 %, error past 90 %. */
export function meterClass(fill: number): string {
  if (fill > 0.9) return "card-meter error";
  if (fill > 0.7) return "card-meter warning";
  return "card-meter";
}

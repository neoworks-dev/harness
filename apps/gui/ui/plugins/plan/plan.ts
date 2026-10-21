// The agent's plan, as the lists its events leave: every `plan` event replaces that agent's list
// (no `agent` is the session's own; an empty one clears it). Ported from the terminal app's
// plan widget.
import type { Plan, PlanEntryStatus } from "@neoworks-dev/neo-protocol/types";

export interface PlanItem {
  text: string;
  status: PlanEntryStatus;
}

export interface PlanList {
  /** Absent for the session's own agent. */
  agent?: string;
  items: readonly PlanItem[];
}

/** The lists after a `plan` event. Insertion order is kept; `ownFirst` puts the session's own first. */
export function applyPlan(lists: readonly PlanList[], plan: Plan): readonly PlanList[] {
  const items = plan.entries.map((entry) => ({ text: entry.content, status: entry.status }));
  const at = lists.findIndex((list) => list.agent === plan.agent);
  if (items.length === 0) return at < 0 ? lists : lists.filter((_, index) => index !== at);
  const next: PlanList = plan.agent === undefined ? { items } : { agent: plan.agent, items };
  if (at < 0) return [...lists, next];
  return lists.map((list, index) => (index === at ? next : list));
}

/** The session's own list first, then the guests' in the order they appeared. */
export function ownFirst(lists: readonly PlanList[]): PlanList[] {
  return [
    ...lists.filter((l) => l.agent === undefined),
    ...lists.filter((l) => l.agent !== undefined),
  ];
}

export interface PlanProgress {
  done: number;
  total: number;
}

/** How many entries of all lists are completed. */
export function progress(lists: readonly PlanList[]): PlanProgress {
  const items = lists.flatMap((list) => list.items);
  return { done: items.filter((item) => item.status === "completed").length, total: items.length };
}

/** ✓ done, ◐ in progress, ○ to do. */
export function glyphOf(status: PlanEntryStatus): string {
  if (status === "completed") return "✓";
  if (status === "in_progress") return "◐";
  return "○";
}

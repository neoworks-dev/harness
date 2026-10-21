import { expect, test } from "bun:test";
import type { Plan } from "@neoworks-dev/neo-protocol/types";
import { applyPlan, glyphOf, ownFirst, progress, type PlanList } from "../ui/plugins/plan/plan.ts";

function plan(
  agent: string | undefined,
  ...entries: [string, Plan["entries"][number]["status"]][]
): Plan {
  const event: Plan = {
    type: "plan",
    entries: entries.map(([content, status]) => ({ content, status })),
  };
  if (agent !== undefined) event.agent = agent;
  return event;
}

test("every plan event replaces that agent's list; an empty one clears it", () => {
  let lists: readonly PlanList[] = [];
  lists = applyPlan(lists, plan("claude", ["c1", "pending"]));
  lists = applyPlan(lists, plan(undefined, ["a", "completed"], ["b", "in_progress"]));
  expect(lists.map((l) => l.agent)).toEqual(["claude", undefined]);
  expect(ownFirst(lists).map((l) => l.agent)).toEqual([undefined, "claude"]);
  lists = applyPlan(
    lists,
    plan(undefined, ["a", "completed"], ["b", "completed"], ["c", "pending"]),
  );
  expect(lists[1]?.items).toHaveLength(3);
  expect(progress(lists)).toEqual({ done: 2, total: 4 });
  lists = applyPlan(lists, plan("claude"));
  expect(lists.map((l) => l.agent)).toEqual([undefined]);
  expect(applyPlan(lists, plan("neo"))).toBe(lists);
  expect(glyphOf("completed") + glyphOf("in_progress") + glyphOf("pending")).toBe("✓◐○");
});

import { expect, test } from "bun:test";
import type { SessionEvent, SessionInfo } from "@suite/protocol/types";
import {
  LINGER_MS,
  activityOf,
  applyAgentEvent,
  compact,
  elapsed,
  firstSelection,
  modelLabel,
  moveAgent,
  shownAgents,
  type AgentRow,
} from "../ui/plugins/agents/agents.ts";

let seq = 0;
function fold(rows: readonly AgentRow[], ts: number, event: SessionEvent): readonly AgentRow[] {
  return applyAgentEvent(rows, { session: "s1", seq: ++seq, ts, event });
}

const started = (id: string, agent: string, extra: Partial<SessionEvent> = {}): SessionEvent =>
  ({ type: "subagent_started", id, agent, task: "do it", ...extra }) as SessionEvent;

test("a row says what the agent does, its tokens and how long", () => {
  let rows = fold([], 1000, started("d1", "neo", { model: "deepseek/deepseek-v4-flash" }));
  expect(rows[0]).toMatchObject({ harness: "neo", state: "running", activity: "starting…" });
  rows = fold(rows, 2000, {
    type: "agent_thought_chunk",
    messageId: "neo-1",
    text: "hm",
    agent: "neo",
  });
  expect(rows[0]?.activity).toBe("thinking…");
  rows = fold(rows, 3000, {
    type: "tool_call",
    toolCallId: "neo:t1",
    title: "search tools: weather in Berlin",
    kind: "search",
    status: "in_progress",
    agent: "neo",
  });
  rows = fold(rows, 4000, { type: "usage", agent: "neo", inputTokens: 3000, outputTokens: 412 });
  expect(rows[0]).toMatchObject({ activity: "search tools: weather in Berlin", tokens: 3412 });
  // Events of another agent's name, or of the session's own agent, don't touch it.
  const other = fold(rows, 5000, {
    type: "tool_call",
    toolCallId: "x:1",
    title: "other",
    kind: "other",
    status: "pending",
    agent: "claude",
  });
  const own = fold(rows, 6000, {
    type: "tool_call",
    toolCallId: "m",
    title: "bash: ls",
    kind: "execute",
    status: "pending",
  });
  expect(other).toBe(rows);
  expect(own).toBe(rows);
  // Without input or output tokens the context size stands in.
  rows = fold(rows, 7000, { type: "usage", agent: "neo", contextUsed: 900 });
  expect(rows[0]?.tokens).toBe(900);
});

test("a finished row shows its outcome for five seconds", () => {
  let rows: readonly AgentRow[] = [];
  for (const [id, agent] of [
    ["d1", "neo"],
    ["d2", "claude"],
    ["d3", "codex"],
  ] as const) {
    rows = fold(rows, 0, started(id, agent));
  }
  rows = fold(rows, 10_000, { type: "subagent_ended", id: "d1", stopReason: "end_turn" });
  rows = fold(rows, 11_000, { type: "subagent_ended", id: "d2", stopReason: "cancelled" });
  rows = fold(rows, 12_000, {
    type: "subagent_ended",
    id: "d3",
    stopReason: "end_turn",
    error: "model refused",
  });
  expect(rows.map((r) => [r.state, r.activity])).toEqual([
    ["done", "done"],
    ["stopped", "stopped"],
    ["done", "done"],
  ]);
  expect(shownAgents(rows, 13_000)).toHaveLength(3);
  expect(shownAgents(rows, 10_000 + LINGER_MS).map((r) => r.id)).toEqual(["d2", "d3"]);
  expect(shownAgents(rows, 20_000)).toHaveLength(0);
  // An error with another stop reason is a failure that says why.
  const failed = fold(fold([], 0, started("e1", "neo")), 5, {
    type: "subagent_ended",
    id: "e1",
    stopReason: "refusal",
    error: "model refused",
  });
  expect(failed[0]).toMatchObject({ state: "failed", activity: "model refused" });
});

test("the same delegation twice is one row", () => {
  const once = fold([], 0, started("d1", "neo"));
  expect(fold(once, 5, started("d1", "neo"))).toBe(once);
});

test("a turn that ends interrupts what still runs", () => {
  let rows = fold([], 0, started("d1", "neo"));
  rows = fold(rows, 1000, { type: "turn_ended", stopReason: "end_turn" });
  expect(rows[0]).toMatchObject({ state: "failed", activity: "interrupted", endedMs: 1000 });
  expect(fold(rows, 2000, { type: "turn_ended", stopReason: "end_turn" })).toBe(rows);
});

test("an agent started in the background runs on past the end of a turn", () => {
  let rows = fold([], 0, started("d1", "claude", { background: true }));
  rows = fold(rows, 10, started("d2", "neo"));
  rows = fold(rows, 1000, { type: "turn_ended", stopReason: "end_turn" });
  expect(rows.map((r) => r.state)).toEqual(["running", "failed"]);
  expect(rows[0]).toMatchObject({ background: true });
});

test("a message to or from a running agent is what it does now", () => {
  let rows = fold([], 0, started("d1", "claude", { background: true }));
  rows = fold(rows, 1, { type: "agent_message", from: "main", to: "d1", text: "hi" });
  expect(rows[0]?.activity).toBe("← main");
  rows = fold(rows, 2, { type: "agent_message", from: "d1", to: "main", text: "hi" });
  expect(rows[0]?.activity).toBe("→ main");
});

test("an agent in a directory of its own reports what its session says", () => {
  const rows = fold([], 0, started("d1", "neo", { child: "s_child", cwd: "/w/site/.worktrees/x" }));
  const sessions = [
    { id: "s_child", preview: "Editing\n the form", cwd: "/w", state: "running" },
  ] as unknown as SessionInfo[];
  expect(activityOf(rows[0] as AgentRow, sessions)).toBe("Editing the form");
  expect(activityOf(rows[0] as AgentRow, [])).toBe("starting…");
  // Labelled events of the harness's name belong to agents that share the session's directory.
  const labelled = fold(rows, 5, {
    type: "agent_thought_chunk",
    messageId: "m",
    text: "x",
    agent: "neo",
  });
  expect(labelled).toBe(rows);
});

test("durations, tokens and models read short", () => {
  expect(elapsed(5_000)).toBe("5s");
  expect(elapsed(72_000)).toBe("1m 12s");
  expect(elapsed(2 * 3_600_000 + 3 * 60_000)).toBe("2h 03m");
  expect(elapsed(-5)).toBe("0s");
  expect(compact(812)).toBe("812");
  expect(compact(3412)).toBe("3.4k");
  expect(compact(1_200_000)).toBe("1.2M");
  expect(modelLabel("deepseek/deepseek-v4-flash")).toBe("deepseek-v4-flash");
  expect(modelLabel("sonnet")).toBe("sonnet");
});

test("the keys start on the latest running row and stop at the ends", () => {
  let rows: readonly AgentRow[] = [];
  for (const id of ["a", "b", "c"]) rows = fold(rows, 0, started(id, id));
  rows = fold(rows, 5, { type: "subagent_ended", id: "c", stopReason: "end_turn" });
  expect(firstSelection(rows)).toBe("b");
  expect(firstSelection(rows.map((r) => ({ ...r, state: "done" as const })))).toBe("c");
  expect(firstSelection([])).toBeUndefined();
  expect(moveAgent(rows, "b", 1)).toBe("c");
  expect(moveAgent(rows, "c", 1)).toBe("c");
  expect(moveAgent(rows, "a", -1)).toBe("a");
  expect(moveAgent(rows, "gone", 1)).toBe("b");
  expect(moveAgent([], "a", 1)).toBeUndefined();
});

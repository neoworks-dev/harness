import { expect, test } from "bun:test";
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { MAX_ROWS, statusOf, summarize } from "../src/summary.ts";
import { costLabel, headline, meterClass, statusLabel, todayLabel } from "../widget/view.ts";

const NOW = new Date(2026, 10, 9, 15, 0);
const TODAY = new Date(2026, 10, 9, 9, 0).getTime();
const YESTERDAY = new Date(2026, 10, 8, 22, 0).getTime();

function session(id: string, fields: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    cwd: "/w",
    createdAt: 0,
    lastActivity: TODAY,
    lastSeq: 1,
    state: "idle",
    queued: 0,
    ...fields,
  };
}

test("a session's status: asking over running, subagents still working, unseen when done", () => {
  expect(statusOf(session("a", { state: "running", asking: 1 }))).toBe("asking");
  expect(statusOf(session("a", { state: "running" }))).toBe("working");
  expect(statusOf(session("a", { state: "running", waitingOn: ["review"] }))).toBe("waiting");
  expect(statusOf(session("a", { unseen: true }))).toBe("unseen");
  expect(statusOf(session("a"))).toBe("idle");
});

test("rows put what needs the user first, then work, then the most recent", () => {
  const { agents } = summarize(
    [
      session("old", { lastActivity: YESTERDAY }),
      session("recent", { lastActivity: TODAY + 10 }),
      session("busy", { state: "running" }),
      session("ask", { asking: 2 }),
    ],
    NOW,
    true,
  );
  expect(agents.map((row) => row.id)).toEqual(["ask", "busy", "recent", "old"]);
});

test("subagent sessions fold into their parent's row: cost, running count and questions", () => {
  const { agents, usage } = summarize(
    [
      session("parent", {
        state: "running",
        usage: { cost: 1, inputTokens: 0, outputTokens: 0, contextUsed: 50, contextSize: 200 },
      }),
      session("child", {
        parent: "parent",
        state: "running",
        usage: { cost: 0.5, inputTokens: 0, outputTokens: 0 },
      }),
      session("asker", { parent: "parent", asking: 1 }),
    ],
    NOW,
    true,
  );
  expect(agents).toHaveLength(1);
  expect(agents[0]).toMatchObject({
    id: "parent",
    status: "asking",
    cost: 1.5,
    subagents: 1,
    contextFill: 0.25,
  });
  expect(usage).toEqual({
    connected: true,
    costToday: 1.5,
    sessionsToday: 1,
    working: 0,
    asking: 1,
  });
});

test("today counts sessions active since local midnight, and rows are capped", () => {
  const sessions = Array.from({ length: MAX_ROWS + 3 }, (_, index) =>
    session(`s${index}`, {
      lastActivity: index === 0 ? YESTERDAY : TODAY,
      usage: { cost: 1, inputTokens: 0, outputTokens: 0 },
    }),
  );
  const { agents, usage } = summarize(sessions, NOW, true);
  expect(agents).toHaveLength(MAX_ROWS);
  expect(usage.costToday).toBe(MAX_ROWS + 2);
  expect(usage.sessionsToday).toBe(MAX_ROWS + 2);
});

test("the widget's words", () => {
  const usage = { connected: true, costToday: 0.004, sessionsToday: 1, working: 2, asking: 1 };
  expect(headline(usage)).toBe("1 need you · 2 working");
  expect(headline({ ...usage, working: 0, asking: 0 })).toBe("All quiet");
  expect(headline({ ...usage, connected: false })).toBe("neo isn't running");
  expect(todayLabel(usage)).toBe("<$0.01 today · 1 session");
  expect(costLabel(12.345)).toBe("$12.35");
  expect(meterClass(0.95)).toBe("card-meter error");
  expect(meterClass(0.8)).toBe("card-meter warning");
  const { agents } = summarize(
    [session("a", { state: "running" }), session("b", { parent: "a", state: "running" })],
    NOW,
    true,
  );
  expect(agents.map(statusLabel)).toEqual(["working · 1 subagents"]);
});

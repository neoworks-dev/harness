// How messages between agents are drawn: compact rows in the chat, hidden background work, and
// the per-agent transcripts.
import { describe, expect, test } from "bun:test";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { silentLogger } from "../src/log.ts";
import { SessionView } from "../src/view/session-view.ts";

function feed(events: SessionEvent[], agent?: string): SessionView {
  const view = new SessionView("s", () => undefined, silentLogger, agent);
  for (const [i, event] of events.entries()) view.apply({ session: "s", seq: i + 1, ts: 0, event });
  return view;
}

const started: SessionEvent = {
  type: "subagent_started",
  id: "d_1",
  agent: "claude",
  task: "fix it",
  background: true,
};
const foreground: SessionEvent = {
  type: "subagent_started",
  id: "d_1",
  agent: "claude",
  task: "fix it",
};
const ask: SessionEvent = {
  type: "agent_message",
  from: "d_1",
  fromAgent: "claude",
  to: "main",
  text: "Which database?",
};
const answer: SessionEvent = {
  type: "agent_message",
  from: "main",
  to: "d_1",
  toAgent: "claude",
  text: "Staging.",
};
const reply: SessionEvent = {
  type: "agent_message",
  from: "d_1",
  fromAgent: "claude",
  to: "main",
  text: "Done.",
  reply: true,
};
const turnEnd: SessionEvent = { type: "turn_ended", stopReason: "end_turn" };
const working: SessionEvent = {
  type: "agent_message_chunk",
  messageId: "m-1",
  text: "working",
  agent: "claude",
};

describe("in the chat", () => {
  test("a message is a compact row, labelled by who told whom", () => {
    const items = feed([started, ask, answer, reply]).snapshot().items;
    expect(items.map((i) => [i.kind, i.state, i.label])).toEqual([
      ["tool", "done", "message: claude d_1 → main"],
      ["tool", "done", "message: main → claude d_1"],
      ["tool", "done", "reply: claude d_1 → main"],
    ]);
    expect(items[0]?.component).toMatchObject({ type: "text" });
  });

  test("the turn a message starts shows as its row, not as something the user said", () => {
    const items = feed([
      ask,
      { type: "user_message", app: "agents", text: '<message from="d_1">…</message>' },
      { type: "user_message", app: "cli", text: "thanks" },
    ]).snapshot().items;
    expect(items.map((i) => i.kind)).toEqual(["tool", "user"]);
  });

  test("a background subagent stays hidden past the end of a turn, until it ends", () => {
    expect(feed([started, turnEnd, working]).snapshot().items).toEqual([]);
    const ended: SessionEvent = { type: "subagent_ended", id: "d_1", stopReason: "end_turn" };
    expect(
      feed([started, turnEnd, ended, working])
        .snapshot()
        .items.map((i) => i.kind),
    ).toEqual(["agent"]);
  });

  test("one the agent waited for is closed by the turn's end, as before", () => {
    const items = feed([foreground, turnEnd, working]).snapshot().items;
    expect(items.map((i) => i.kind)).toEqual(["agent"]);
  });
});

describe("a message the agent sends with a tool", () => {
  // Claude Code's own subagent, then its SendMessage to it, which runs it again.
  const explore: SessionEvent = {
    type: "subagent_started",
    id: "c-1",
    agent: "claude/1a2b3c4d",
    name: "Explore",
    task: "List them.",
    native: true,
    handle: "a7e6",
  };
  const ended = (id: string): SessionEvent => ({
    type: "subagent_ended",
    id,
    stopReason: "end_turn",
  });
  const send = (status: "completed" | "failed", result: string): SessionEvent[] => [
    {
      type: "tool_call",
      toolCallId: "t-send",
      title: "SendMessage",
      kind: "other",
      status: "pending",
      rawInput: { to: "a7e6", message: "Now count them." },
    },
    {
      type: "tool_call_update",
      toolCallId: "t-send",
      status,
      content: [{ type: "text", text: result }],
    },
  ];
  const said = (text: string): SessionEvent => ({
    type: "agent_message_chunk",
    messageId: "m-2",
    text,
    agent: "claude/1a2b3c4d",
  });
  const json = (item: unknown): string => JSON.stringify(item);

  test("names the subagent, shows the message, and its answer once that run ends", () => {
    const before = feed([explore, ended("c-1"), ...send("completed", '{"success":true}')]);
    const item = before.snapshot().items.find((i) => i.ref === "c-t-send");
    expect(item?.label).toBe("message Explore");
    expect(json(item)).toContain("Now count them.");
    expect(json(item)).not.toContain("success");

    const after = feed([
      explore,
      ended("c-1"),
      ...send("completed", '{"success":true}'),
      { ...explore, id: "t-send" },
      said("Found 3 tests."),
      ended("t-send"),
    ]);
    const answeredItem = after.snapshot().items.find((i) => i.ref === "c-t-send");
    expect(json(answeredItem)).toContain("Found 3 tests.");
  });

  test("one that didn't arrive says why and waits for nothing", () => {
    const refused = '{"success":false,"message":"Agent was stopped by the user"}';
    const view = feed([
      explore,
      ended("c-1"),
      ...send("completed", refused),
      { ...explore, id: "t-send" },
      said("unrelated"),
      ended("t-send"),
    ]);
    const item = view.snapshot().items.find((i) => i.ref === "c-t-send");
    expect(json(item)).toContain("Agent was stopped by the user");
    expect(json(item)).not.toContain("unrelated");
  });

  test("neo's send_message to a delegated subagent gets its reply, not a row of its own", () => {
    const items = feed([
      started,
      {
        type: "tool_call",
        toolCallId: "t-1",
        title: "message d_1: Use staging",
        kind: "other",
        status: "completed",
        rawInput: { to: "d_1", text: "Use staging." },
      },
      reply,
    ]).snapshot().items;
    expect(items.map((i) => i.ref)).toEqual(["c-t-1"]);
    expect(json(items[0])).toContain("Done.");
  });
});

describe("in an agent's transcript", () => {
  test("what was written to it is input like its task; what it sent is a row", () => {
    const items = feed([started, answer, ask], "claude").snapshot().items;
    expect(items.map((i) => [i.kind, i.label])).toEqual([
      ["user", "task"],
      ["user", "from main"],
      ["tool", "message: claude d_1 → main"],
    ]);
  });

  test("its final reply is not repeated; another agent's messages are not here", () => {
    expect(
      feed([started, reply], "claude")
        .snapshot()
        .items.map((i) => i.label),
    ).toEqual(["task"]);
    expect(feed([started, answer], "neo").snapshot().items).toEqual([]);
  });
});

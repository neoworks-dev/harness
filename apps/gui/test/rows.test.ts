import { expect, test } from "bun:test";
import type { Component, SessionEvent, ViewItem } from "@neoworks-dev/neo-protocol/types";
import {
  buildRows,
  splitMessages,
  type Row,
  type RowsInput,
} from "../ui/plugins/transcript/rows.ts";
import { applyRecord, emptyTimeline, type Timeline } from "../ui/plugins/transcript/timeline.ts";

const CWD = "/w";

const text = (value: string): Component => ({
  type: "text",
  blocks: [{ type: "paragraph", spans: [{ text: value }] }],
});
const user = (seq: number, value = "do it"): ViewItem => ({
  ref: `u${seq}`,
  kind: "user",
  label: "cli",
  component: text(value),
});
const agent = (id: string, value = "ok"): ViewItem => ({
  ref: `m-${id}`,
  kind: "agent",
  component: text(value),
});
const thought = (id: string): ViewItem => ({
  ref: `t-${id}`,
  kind: "thought",
  component: text("Hmm."),
});
const bash = (id: string, state: ViewItem["state"] = "done"): ViewItem => ({
  ref: `c-${id}`,
  kind: "tool",
  label: `bash: ${id}`,
  state,
  component: { type: "code", code: "out" },
});
const edit = (id: string, path: string, state: ViewItem["state"] = "done"): ViewItem => ({
  ref: `c-${id}`,
  kind: "tool",
  label: `edit ${path}`,
  state,
  component: {
    type: "diff",
    path: `${CWD}/${path}`,
    lines: [
      { kind: "hunk", text: "@@" },
      { kind: "add", text: "x", new: 1 },
    ],
  },
});
const notice = (ref: string, value = "n"): ViewItem => ({
  ref,
  kind: "notice",
  component: text(value),
});

/** Feeds events in, one per second from t=1000, as the daemon would number them. */
function timelineOf(events: SessionEvent[], from = 1): Timeline {
  const timeline = emptyTimeline();
  events.forEach((event, index) => {
    applyRecord(timeline, { session: "s", seq: from + index, ts: 1000 * (index + 1), event });
  });
  return timeline;
}

function rows(items: ViewItem[], options: Partial<RowsInput> = {}): Row[] {
  return buildRows({
    items,
    timeline: emptyTimeline(),
    cwd: CWD,
    running: false,
    isOpen: () => false,
    ...options,
  });
}

const kinds = (list: Row[]): string[] => list.map((row) => row.kind);

test("without turns, items are rows of their own", () => {
  expect(kinds(rows([agent("a"), notice("n1")]))).toEqual(["message", "item"]);
  expect(rows([])).toEqual([]);
});

test("a turn's tool calls merge into one group, broken by a message", () => {
  const list = rows(
    [user(1), bash("a"), bash("b"), thought("x"), agent("mid"), bash("c"), agent("end")],
    { running: true },
  );
  expect(kinds(list)).toEqual(["item", "work", "message", "work", "message", "working"]);
  const first = list[1];
  expect(first?.kind === "work" && first.entries.map((e) => e.kind)).toEqual([
    "command",
    "command",
    "thought",
  ]);
});

test("a running turn is never folded, and shows its clock", () => {
  const list = rows([user(1), bash("a"), agent("end")], { running: true, startedAt: 5 });
  expect(kinds(list)).toEqual(["item", "work", "message", "working"]);
  expect(list.at(-1)).toMatchObject({ kind: "working", startedAt: 5 });
  const message = list[2];
  expect(message?.kind === "message" && message.final).toBe(false);
});

test("a finished turn folds its work under one row, closed", () => {
  const events: SessionEvent[] = [
    { type: "user_message", app: "cli", text: "do it" },
    { type: "tool_call", toolCallId: "a", title: "bash: a", kind: "execute", status: "pending" },
    { type: "agent_message_chunk", messageId: "end", text: "ok" },
    { type: "turn_ended", stopReason: "end_turn" },
  ];
  const list = rows([user(1), bash("a"), agent("mid"), bash("b"), agent("end")], {
    timeline: timelineOf(events),
  });
  expect(kinds(list)).toEqual(["item", "fold", "message"]);
  expect(list[1]).toMatchObject({ kind: "fold", open: false, label: "Worked for 3s" });
  expect(list[2]).toMatchObject({ kind: "message", final: true });
});

test("an open fold shows its rows in place, grouped", () => {
  const list = rows([user(1), bash("a"), bash("b"), agent("mid"), bash("c"), agent("end")], {
    isOpen: (key) => key === "fold:u1",
  });
  expect(kinds(list)).toEqual(["item", "fold", "work", "message", "work", "message"]);
  expect(list[1]).toMatchObject({ open: true, label: "Worked" });
});

test("the duration comes from the first and last item when the turn's end is unknown", () => {
  const timeline = timelineOf([
    { type: "user_message", app: "cli", text: "x" },
    { type: "tool_call", toolCallId: "a", title: "bash: a", kind: "execute", status: "pending" },
    { type: "agent_message_chunk", messageId: "end", text: "ok" },
  ]);
  const list = rows([user(1), bash("a"), agent("end")], { timeline });
  expect(list[1]).toMatchObject({ kind: "fold", label: "Worked for 1s" });
});

test("a turn with a failed call is not folded", () => {
  const list = rows([user(1), bash("a", "failed"), agent("end")]);
  expect(kinds(list)).toEqual(["item", "work", "message"]);
  expect(list[2]).toMatchObject({ final: true });
});

test("a refused or failed turn is not folded", () => {
  const timeline = timelineOf([
    { type: "user_message", app: "cli", text: "x" },
    { type: "turn_ended", stopReason: "error", error: "boom" },
  ]);
  const list = rows([user(1), bash("a"), agent("end"), notice("e2", "boom")], { timeline });
  expect(kinds(list)).toEqual(["item", "work", "message", "item"]);
});

test("a stopped turn is not folded and says how long it ran", () => {
  const timeline = timelineOf([
    { type: "user_message", app: "cli", text: "x" },
    { type: "tool_call", toolCallId: "a", title: "bash: a", kind: "execute", status: "pending" },
    { type: "turn_ended", stopReason: "cancelled" },
  ]);
  const list = rows([user(1), bash("a"), notice("e3", "stopped: cancelled")], { timeline });
  expect(kinds(list)).toEqual(["item", "work", "stopped"]);
  expect(list[2]).toMatchObject({ label: "You stopped after 2s" });
});

test("a finished turn with no reply folds all of its work", () => {
  const list = rows([user(1), bash("a"), bash("b")]);
  expect(kinds(list)).toEqual(["item", "fold"]);
});

test("a turn with nothing to hide has no fold", () => {
  expect(kinds(rows([user(1), agent("end")]))).toEqual(["item", "message"]);
});

test("notices stay in view when their turn folds", () => {
  const list = rows([user(1), bash("a"), notice("n2", "claude connected"), agent("end")]);
  expect(kinds(list)).toEqual(["item", "fold", "item", "message"]);
});

test("the final reply carries the files the turn changed", () => {
  const list = rows([
    user(1),
    edit("a", "a.ts"),
    edit("b", "a.ts"),
    edit("c", "b.ts"),
    agent("end"),
  ]);
  const message = list.at(-1);
  expect(message?.kind === "message" && message.changes.map((f) => [f.path, f.added])).toEqual([
    ["a.ts", 2],
    ["b.ts", 1],
  ]);
});

test("a plan is a row of its own, and the call that wrote it is dropped", () => {
  const plan: ViewItem = {
    ref: "plan",
    kind: "tool",
    label: "Plan",
    state: "running",
    component: text("x"),
  };
  const call: ViewItem = {
    ref: "c-todo",
    kind: "tool",
    label: "update todos",
    state: "done",
    component: text("ok"),
  };
  const list = rows([user(1), bash("a"), call, plan, bash("b")], { running: true });
  expect(kinds(list)).toEqual(["item", "work", "work", "work", "working"]);
  const middle = list[2];
  expect(middle?.kind === "work" && middle.entries[0]?.kind).toBe("plan");
});

test("a question is a row of its own and splits a group", () => {
  const question: ViewItem = {
    ref: "p-1",
    kind: "tool",
    label: "claude asks: rm",
    state: "pending",
    component: { type: "button", label: "Allow", action: "suite.permit" },
  };
  const list = rows([user(1), bash("a"), question, bash("b")], { running: true });
  expect(kinds(list)).toEqual(["item", "work", "work", "work", "working"]);
});

test("the last thought of a running turn is still being written", () => {
  const list = rows([user(1), thought("x")], { running: true });
  const work = list[1];
  expect(work?.kind === "work" && work.entries[0]).toMatchObject({ live: true, label: "Thinking" });
  const done = rows([user(1), thought("x"), bash("a")], { running: true })[1];
  expect(done?.kind === "work" && done.entries[0]?.live).toBe(false);
});

test("each turn is folded on its own, keys stay put as a turn grows", () => {
  const first = [user(1), bash("a"), agent("one")];
  const second = [user(4), bash("b")];
  const list = rows([...first, ...second], { running: true });
  expect(kinds(list)).toEqual(["item", "fold", "message", "item", "work", "working"]);
  const keys = list.map((row) => row.key);
  expect(new Set(keys).size).toBe(keys.length);
  const grown = rows([...first, ...second, bash("c")], { running: true });
  expect(grown.find((row) => row.kind === "work")?.key).toBe("work:c-b");
});

test("the timeline learns turns, stamps and facts from events", () => {
  const timeline = timelineOf([
    { type: "user_message", app: "cli", text: "x" },
    { type: "tool_call", toolCallId: "a", title: "bash", kind: "execute", status: "pending" },
    { type: "tool_call_update", toolCallId: "a", rawInput: { command: "ls" }, tool: "bash" },
    { type: "agent_message_chunk", messageId: "m", text: "a" },
    { type: "agent_message_chunk", messageId: "m", text: "b" },
    { type: "plan", entries: [] },
    { type: "turn_ended", stopReason: "cancelled" },
  ]);
  expect(timeline.stamps.get("m-m")).toEqual({ first: 4000, last: 5000 });
  expect(timeline.stamps.get("c-a")?.first).toBe(2000);
  expect(timeline.facts.get("c-a")).toMatchObject({
    kind: "execute",
    tool: "bash",
    rawInput: { command: "ls" },
  });
  expect(timeline.turns.get("u1")).toMatchObject({
    startedAt: 1000,
    endedAt: 7000,
    stopReason: "cancelled",
    noticeRef: "e7",
  });
  expect(timeline.current).toBeUndefined();
  expect(timeline.stamps.has("plan")).toBe(true);
});

test("a reply of text becomes a row per block and then its own row, the rest stays", () => {
  const long: ViewItem = {
    ref: "m-long",
    kind: "agent",
    label: "claude",
    component: {
      type: "text",
      blocks: [
        { type: "paragraph", spans: [{ text: "one" }] },
        { type: "rule" },
        { type: "paragraph", spans: [{ text: "two" }] },
      ],
    },
  };
  const card: ViewItem = { ref: "m-card", kind: "agent", component: { type: "code", code: "x" } };
  // A running turn, so nothing folds away.
  const split = splitMessages(rows([user(1), long, card], { running: true }));
  expect(split.map((row) => [row.kind, row.key])).toEqual([
    ["item", "u1"],
    ["prose", "m-long/0"],
    ["prose", "m-long/1"],
    ["prose", "m-long/2"],
    ["message", "m-long"],
    ["message", "m-card"],
    ["working", "working"],
  ]);
  expect(split[1]).toMatchObject({ first: true, label: "claude", ref: "m-long" });
  expect(split[2]).toMatchObject({ first: false });
  expect(split[2]).not.toHaveProperty("label");
  expect(split[4]).toMatchObject({ split: true });
  expect(split[5]).not.toHaveProperty("split");
});

test("a block that stays the same keeps its row object as the reply grows", () => {
  const first = { type: "paragraph" as const, spans: [{ text: "settled" }] };
  const reply = (last: string): ViewItem => ({
    ref: "m-a",
    kind: "agent",
    component: { type: "text", blocks: [first, { type: "paragraph", spans: [{ text: last }] }] },
  });
  const before = splitMessages(rows([reply("wri")]));
  const after = splitMessages(rows([reply("writing")]));
  expect(after[0]).toBe(before[0]);
  expect(after[1]).not.toBe(before[1]);
  expect(after[1]).toMatchObject({ key: "m-a/1" });
});

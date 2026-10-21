import { expect, test } from "bun:test";
import type { SessionEvent, SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import { lineDiff, splitLines } from "../ui/plugins/viewer/linediff.ts";
import { NO_TURN, applyTurn, turnDiffs, turnEdits, type Turn } from "../ui/plugins/viewer/turn.ts";

/** The lines as "-old", "+new", " same", to compare at a glance. */
function marks(oldText: string | undefined, newText: string): string[] {
  const sign = { add: "+", remove: "-", context: " ", hunk: "@" } as const;
  return lineDiff(oldText, newText).map((line) => `${sign[line.kind]}${line.text}`);
}

test("a text's lines do not include the one a final newline would start", () => {
  expect(splitLines("a\nb\n")).toEqual(["a", "b"]);
  expect(splitLines("a\n\nb")).toEqual(["a", "", "b"]);
  expect(splitLines("")).toEqual([]);
});

test("changed lines are removed and added, the rest is context, numbered in their own text", () => {
  expect(marks("one\ntwo\n", "one\nthree\n")).toEqual([" one", "-two", "+three"]);
  const lines = lineDiff("a\nb\nc\n", "a\nx\ny\nc\n");
  expect(lines.map((line) => [line.old, line.new])).toEqual([
    [1, 1],
    [2, undefined],
    [undefined, 2],
    [undefined, 3],
    [3, 4],
  ]);
});

test("lines kept between changes stay context", () => {
  expect(marks("a\nb\nc\nd\ne\n", "a\nB\nc\nD\ne\n")).toEqual([
    " a",
    "-b",
    "+B",
    " c",
    "-d",
    "+D",
    " e",
  ]);
  expect(marks("a\nb\nc\n", "a\nc\nd\n")).toEqual([" a", "-b", " c", "+d"]);
});

test("a new file is all added, a deleted one all removed, equal texts all context", () => {
  expect(marks(undefined, "x\ny\n")).toEqual(["+x", "+y"]);
  expect(marks("x\ny\n", "")).toEqual(["-x", "-y"]);
  expect(marks("same\n", "same\n")).toEqual([" same"]);
});

test("a huge change is replaced whole instead of searched", () => {
  const old = Array.from({ length: 3000 }, (_, i) => `a${i}`).join("\n");
  const next = Array.from({ length: 3000 }, (_, i) => `b${i}`).join("\n");
  const lines = lineDiff(old, next);
  expect(lines.filter((line) => line.kind === "remove")).toHaveLength(3000);
  expect(lines.filter((line) => line.kind === "add")).toHaveLength(3000);
});

let seq = 0;
const record = (event: SessionEvent): SessionEventRecord =>
  ({ session: "s", seq: (seq += 1), event }) as SessionEventRecord;

const edit = (
  id: string,
  path: string,
  oldText: string | undefined,
  newText: string,
): SessionEventRecord =>
  record({
    type: "tool_call_update",
    toolCallId: id,
    content: [{ type: "diff", path, oldText, newText }],
  });

function fold(events: SessionEventRecord[], cwd = "/work"): Turn {
  return events.reduce((turn, event) => applyTurn(turn, event, cwd), NO_TURN);
}

test("a turn collects the diffs of its tool calls and starts over with the next message", () => {
  const turn = fold([edit("t1", "/work/a.txt", "one\n", "two\n")]);
  expect(turnEdits(turn)).toEqual([{ path: "a.txt", oldText: "one\n", newText: "two\n" }]);
  const next = applyTurn(
    turn,
    record({ type: "user_message", app: "desktop", text: "more" }),
    "/work",
  );
  expect(turnEdits(next)).toEqual([]);
  expect(next.version).toBeGreaterThan(turn.version);
});

test("an update of a call replaces its earlier diff, a new file has no old text", () => {
  const turn = fold([
    edit("t1", "/work/a.txt", "one\n", "two\n"),
    edit("t1", "/work/a.txt", "one\n", "three\n"),
    edit("t2", "/work/src/new.txt", undefined, "fresh\n"),
  ]);
  expect(turnEdits(turn)).toEqual([
    { path: "a.txt", oldText: "one\n", newText: "three\n" },
    { path: "src/new.txt", newText: "fresh\n" },
  ]);
});

test("events that carry no diff leave the turn as it is, but the end of a turn is noted", () => {
  const turn = fold([edit("t1", "/work/a.txt", "a\n", "b\n")]);
  const same = applyTurn(
    turn,
    record({ type: "tool_call_update", toolCallId: "t3", status: "completed" }),
    "/work",
  );
  expect(same).toBe(turn);
  const ended = applyTurn(turn, record({ type: "turn_ended", stopReason: "end_turn" }), "/work");
  expect(ended.calls).toBe(turn.calls);
  expect(ended.version).toBe(turn.version + 1);
});

test("edits one after another on a file are one diff, and edits that cancel are none", () => {
  const chained = fold([
    edit("t1", "/work/a.txt", "a\nb\n", "a\nc\n"),
    edit("t2", "/work/a.txt", "a\nc\n", "a\nd\n"),
  ]);
  const [file] = turnDiffs(turnEdits(chained));
  expect(file?.path).toBe("a.txt");
  expect(file?.lines.map((line) => `${line.kind}:${line.text}`)).toEqual([
    "context:a",
    "remove:b",
    "add:d",
  ]);
  const undone = fold([
    edit("t1", "/work/a.txt", "a\n", "b\n"),
    edit("t2", "/work/a.txt", "b\n", "a\n"),
  ]);
  expect(turnDiffs(turnEdits(undone))).toEqual([]);
});

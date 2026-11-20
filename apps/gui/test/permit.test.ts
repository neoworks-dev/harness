import { expect, test } from "bun:test";
import type { PermissionOption, SessionEvent } from "@neoworks-dev/neo-protocol/types";
import {
  answers,
  applyPermit,
  nextChangeSpot,
  noteOf,
  runOf,
  spotsOf,
  type PendingEdit,
} from "../ui/plugins/viewer/permit.ts";

const OPTIONS: PermissionOption[] = [
  { id: "always", name: "Always", kind: "allow_always" },
  { id: "yes", name: "Allow", kind: "allow_once" },
  { id: "no", name: "Deny", kind: "reject_once" },
];

const fold = (events: SessionEvent[]): readonly PendingEdit[] =>
  events.reduce<readonly PendingEdit[]>(
    (state, event, at) => applyPermit(state, { session: "s", seq: at + 1, ts: 0, event }, "/w"),
    [],
  );

const request = (id: string, oldText: string | undefined, newText: string): SessionEvent => ({
  type: "permission_request",
  id,
  agent: "main",
  title: "Edit a.ts",
  kind: "edit",
  content: [{ type: "diff", path: "/w/src/a.ts", oldText, newText }],
  options: OPTIONS,
});

test("an edit's request waits until it is answered; one without a diff is not an edit", () => {
  const command: SessionEvent = { type: "permission_request", id: "c", agent: "main", title: "Run ls", options: OPTIONS };
  const waiting = fold([request("1", "a\n", "b\n"), command, request("2", undefined, "x\n")]);
  expect(waiting.map((edit) => edit.id)).toEqual(["1", "2"]);
  expect(waiting[0]).toMatchObject({ ref: "p-1", files: [{ path: "src/a.ts" }] });
  const answered = fold([request("1", "a\n", "b\n"), request("2", undefined, "x\n"), { type: "permission_resolved", id: "1", option: "yes" }]);
  expect(answered.map((edit) => edit.id)).toEqual(["2"]);
});

test("Enter allows once (or keeps), x rejects", () => {
  expect(answers(OPTIONS)).toEqual({ accept: OPTIONS[1], decline: OPTIONS[2] });
  const review: PermissionOption[] = [
    { id: "keep", name: "Keep", kind: "allow_once" },
    { id: "revert", name: "Revert", kind: "reject_once" },
  ];
  expect(answers(review).accept?.id).toBe("keep");
});

test("the cursor jumps between runs of changes, across files", () => {
  const [one] = fold([request("1", "a\nb\nc\nd\n", "a\nB\nc\nD\n")]);
  const files = [...one!.files, ...one!.files];
  // a, -b, +B, c, -d, +D in each file.
  expect(spotsOf(files)).toHaveLength(12);
  expect(nextChangeSpot(files, -1, 1)).toBe(1);
  expect(nextChangeSpot(files, 1, 1)).toBe(4);
  expect(nextChangeSpot(files, 4, 1)).toBe(7);
  expect(nextChangeSpot(files, 7, -1)).toBe(4);
  expect(nextChangeSpot(files, 1, -1)).toBeUndefined();
});

test("a visual run stays within the file the cursor is in", () => {
  const spots = [
    { file: 0, index: 0 },
    { file: 0, index: 1 },
    { file: 1, index: 0 },
    { file: 1, index: 1 },
  ];
  expect(runOf(spots, 3, 2)).toEqual({ file: 1, from: 0, to: 1 });
  expect(runOf(spots, 2, 3)).toEqual({ file: 1, from: 0, to: 1 });
  expect(runOf(spots, 1, 2)).toEqual({ file: 1, from: 0, to: 0 });
});

test("the note quotes each commented line with what was said; none without comments", () => {
  const [one] = fold([request("1", "a\nb\n", "a\nB\n")]);
  const files = one!.files;
  expect(noteOf(files, [])).toBe("");
  const note = noteOf(files, [{ file: 0, from: 2, to: 2, text: "lower case" }]);
  expect(note).toContain("`src/a.ts` line 2");
  expect(note).toContain("B");
  expect(note).toContain("lower case");
});

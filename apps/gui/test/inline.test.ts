import { expect, test } from "bun:test";
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";
import { changedSpan, markSpan, partners, withMarks } from "../ui/plugins/viewer/inline.ts";

test("what changed is what is left once the shared start and end are off, as whole words", () => {
  expect(changedSpan("const limit = 10;", "const limit = 25;")).toEqual({ from: 14, to: 16 });
  // A word that grew is marked whole.
  expect(changedSpan("let limits = 1;", "let limit = 1;")).toEqual({ from: 4, to: 10 });
  expect(changedSpan("foo(a, b)", "foo(a)")).toEqual({ from: 5, to: 8 });
});

test("a line that only lost something has nothing to mark, nor one that changed in most of it", () => {
  expect(changedSpan("foo(a)", "foo(a, b)")).toBeUndefined();
  expect(changedSpan("same", "same")).toBeUndefined();
  expect(changedSpan("alpha beta gamma", "completely different")).toBeUndefined();
});

test("marking cuts the runs at the ends of the span", () => {
  const runs = [{ text: "let ", kind: "keyword" as const }, { text: "x = 10;" }];
  expect(markSpan(runs, { from: 6, to: 7 })).toEqual([
    { text: "let ", kind: "keyword" },
    { text: "x " },
    { text: "=", mark: true },
    { text: " 10;" },
  ]);
  // A span over a whole run, and over the edge between two.
  expect(markSpan(runs, { from: 0, to: 4 })).toEqual([
    { text: "let ", kind: "keyword", mark: true },
    { text: "x = 10;" },
  ]);
  expect(markSpan(runs, { from: 2, to: 5 })).toEqual([
    { text: "le", kind: "keyword" },
    { text: "t ", kind: "keyword", mark: true },
    { text: "x", mark: true },
    { text: " = 10;" },
  ]);
});

test("a line with a partner is marked, with or without the runs of the highlighting", () => {
  expect(withMarks("a = 1", undefined, undefined)).toBeUndefined();
  const plain = withMarks("a = 1", undefined, "a = 2");
  expect(plain).toEqual([{ text: "a = " }, { text: "1", mark: true }]);
  const runs = [{ text: "a = " }, { text: "1", kind: "number" as const }];
  expect(withMarks("a = 1", runs, "a = 2")).toEqual([
    { text: "a = " },
    { text: "1", kind: "number", mark: true },
  ]);
  expect(withMarks("a = 1", runs, "completely other")).toBe(runs);
});

test("removed lines are paired with the added ones that follow, from the top", () => {
  const line = (kind: DiffLine["kind"], text: string): DiffLine => ({ kind, text });
  const lines = [
    line("context", "x"),
    line("remove", "r1"),
    line("remove", "r2"),
    line("remove", "r3"),
    line("add", "a1"),
    line("add", "a2"),
    line("context", "y"),
    line("add", "lonely"),
    line("remove", "gone"),
  ];
  expect([...partners(lines)]).toEqual([
    [1, "a1"],
    [4, "r1"],
    [2, "a2"],
    [5, "r2"],
  ]);
});

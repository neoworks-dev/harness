import { expect, test } from "bun:test";
import type { DiffLine } from "@suite/protocol/types";
import { clip, diffStats, gutterWidth } from "../ui/lib/diff.ts";

const lines: DiffLine[] = [
  { kind: "hunk", text: "@@ -1,4 +1,4 @@" },
  { kind: "context", text: "one", old: 1, new: 1 },
  { kind: "remove", text: "two", old: 2 },
  { kind: "remove", text: "three", old: 3 },
  { kind: "add", text: "TWO", new: 2 },
  { kind: "context", text: "four", old: 4, new: 3 },
  { kind: "add", text: "five", new: 4 },
];

test("a diff counts its added and removed lines", () => {
  expect(diffStats(lines)).toEqual({ added: 2, removed: 2 });
});

test("a long diff is cut to a preview, but never for a single line", () => {
  expect(clip([1, 2, 3, 4, 5], 3)).toEqual({ shown: [1, 2, 3], hidden: 2 });
  expect(clip([1, 2, 3, 4], 3)).toEqual({ shown: [1, 2, 3, 4], hidden: 0 });
});

test("the line-number column is as wide as the largest number", () => {
  expect(gutterWidth(lines)).toBe(1);
  expect(gutterWidth([{ kind: "context", text: "", old: 1200, new: 99 }])).toBe(4);
});

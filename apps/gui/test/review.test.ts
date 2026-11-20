import { expect, test } from "bun:test";
import { nextChange, stepCursor, stepFile } from "../ui/plugins/viewer/review.ts";

test("the cursor moves line by line, stays inside the file, and stretches with shift", () => {
  const start = stepCursor(undefined, "file:a.ts", 10, 1, false);
  expect(start).toEqual({ view: "file:a.ts", anchor: 0, head: 0 });
  const down = stepCursor(start, "file:a.ts", 10, 3, false);
  expect(down).toEqual({ view: "file:a.ts", anchor: 3, head: 3 });
  const stretched = stepCursor(down, "file:a.ts", 10, 2, true);
  expect(stretched).toEqual({ view: "file:a.ts", anchor: 3, head: 5 });
  expect(stepCursor(stretched, "file:a.ts", 10, 99, false)).toEqual({ view: "file:a.ts", anchor: 9, head: 9 });
  expect(stepCursor(down, "file:a.ts", 10, -99, false)).toEqual({ view: "file:a.ts", anchor: 0, head: 0 });
  // Another file: from its top.
  expect(stepCursor(down, "file:b.ts", 10, 1, false)).toEqual({ view: "file:b.ts", anchor: 0, head: 0 });
});

test("n and N jump between runs of changed lines", () => {
  // Lines 3-4 and 9 changed (1-based): runs start at indices 2 and 8.
  const changed = new Set([3, 4, 9]);
  expect(nextChange(changed, -1, 1)).toBe(2);
  expect(nextChange(changed, 2, 1)).toBe(8);
  expect(nextChange(changed, 8, 1)).toBeUndefined();
  expect(nextChange(changed, 8, -1)).toBe(2);
  expect(nextChange(changed, 2, -1)).toBeUndefined();
});

test("] and [ step through the turn's files and stop at its ends", () => {
  expect(stepFile(3, 0, 1)).toBe(1);
  expect(stepFile(3, 2, 1)).toBeUndefined();
  expect(stepFile(3, 0, -1)).toBeUndefined();
});

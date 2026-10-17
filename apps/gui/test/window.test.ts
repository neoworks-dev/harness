import { expect, test } from "bun:test";
import { indexAt, tops, visible } from "../ui/plugins/viewer/window.ts";

const heights = [20, 20, 36, 20, 28];

test("the tops are a running sum, with the list's height last", () => {
  expect(tops(5, (i) => heights[i] ?? 0)).toEqual([0, 20, 40, 76, 96, 124]);
  expect(tops(0, () => 20)).toEqual([0]);
});

test("the row at an offset is the last one that begins at or above it", () => {
  const starts = tops(5, (i) => heights[i] ?? 0);
  expect(indexAt(starts, 0)).toBe(0);
  expect(indexAt(starts, 19.5)).toBe(0);
  expect(indexAt(starts, 20)).toBe(1);
  expect(indexAt(starts, 75)).toBe(2);
  expect(indexAt(starts, 76)).toBe(3);
  expect(indexAt(starts, 9999)).toBe(4);
  expect(indexAt([0], 10)).toBe(0);
});

test("the rows to build are the window's and a few past each edge", () => {
  const starts = tops(100, () => 20);
  expect(visible(starts, 0, 100, 2)).toEqual({ first: 0, last: 8 });
  expect(visible(starts, 1000, 100, 2)).toEqual({ first: 48, last: 58 });
  expect(visible(starts, 5000, 100, 2)).toEqual({ first: 97, last: 100 });
  expect(visible([0], 0, 100, 2)).toEqual({ first: 0, last: 0 });
});

import { expect, test } from "bun:test";
import { addColumn, columnSrc, MAX_COLUMNS } from "../ui/web/columns/columns.ts";

test("a session opens in a column once, never the one the main column shows", () => {
  expect(addColumn([], "a", "main")).toEqual(["a"]);
  expect(addColumn(["a"], "a", "main")).toEqual(["a"]);
  expect(addColumn(["a"], "main", "main")).toEqual(["a"]);
  expect(addColumn(["a"], "b", "main")).toEqual(["a", "b"]);
});

test("past the most columns, the oldest gives way", () => {
  const full = Array.from({ length: MAX_COLUMNS }, (_, i) => `s${i}`);
  expect(addColumn(full, "new", undefined)).toEqual([...full.slice(1), "new"]);
});

test("a column is the same page opened on its session", () => {
  expect(columnSrc("/", "s_ab c")).toBe("/?session=s_ab%20c&column");
});

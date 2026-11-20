import { expect, test } from "bun:test";
import { lastAtOrBefore, Offsets } from "../ui/lib/offsets.ts";

test("an offset names the row there, drawn rows by their height and the rest by the estimate", () => {
  const offsets = new Offsets();
  offsets.set("a", 100);
  offsets.set("c", 10);
  const keys = ["a", "b", "c", "d"];
  const estimate = (): number => 40;
  expect(offsets.indexAt(0, keys, estimate)).toBe(0);
  expect(offsets.indexAt(99, keys, estimate)).toBe(0);
  expect(offsets.indexAt(100, keys, estimate)).toBe(1);
  expect(offsets.indexAt(145, keys, estimate)).toBe(2);
  expect(offsets.indexAt(150, keys, estimate)).toBe(3);
  // Past the end: the last row.
  expect(offsets.indexAt(9999, keys, estimate)).toBe(3);
});

test("the newest command at or above a row", () => {
  const ids = [["c1"], ["r1", "c2", "r2"], [], ["c3"]];
  const command = (id: string): boolean => id.startsWith("c");
  expect(lastAtOrBefore(2, ids, command)).toBe("c2");
  expect(lastAtOrBefore(3, ids, command)).toBe("c3");
  expect(lastAtOrBefore(0, ids, command)).toBe("c1");
  expect(lastAtOrBefore(1, [["r1"], ["r2"]], command)).toBeUndefined();
});

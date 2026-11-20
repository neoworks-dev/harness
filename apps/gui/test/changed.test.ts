import { expect, test } from "bun:test";
import { addedLines } from "../ui/plugins/viewer/changed.ts";

test("the lines an edit wrote, by their number in the new text", () => {
  expect(
    addedLines([
      { kind: "hunk", text: "@@ -1,3 +1,4 @@" },
      { kind: "context", text: "a", old: 1, new: 1 },
      { kind: "remove", text: "b", old: 2 },
      { kind: "add", text: "B", new: 2 },
      { kind: "add", text: "C", new: 3 },
      { kind: "context", text: "d", old: 3, new: 4 },
    ]),
  ).toEqual([2, 3]);
  expect(addedLines([{ kind: "remove", text: "gone", old: 1 }])).toEqual([]);
});

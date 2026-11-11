import { expect, test } from "bun:test";
import type { WaitingInput } from "@neoworks-dev/neo-protocol/types";
import {
  isUsers,
  queueRow,
  queueView,
  redraftImages,
  redraftText,
} from "../ui/plugins/composer/queue.ts";

const user = (text: string, images?: string[]): WaitingInput => ({ app: "desktop", text, images });
const agent = (text: string): WaitingInput => ({ app: "neo", text, agent: true });

test("a row is the first line, with a mark for more lines and for pictures", () => {
  expect(queueRow(user("fix the bug")).label).toBe("◷ fix the bug");
  expect(queueRow(user("\n  first\nsecond")).label).toBe("◷ first …");
  expect(queueRow(user("look", ["/a.png"])).label).toBe("◷ look  [image]");
  expect(queueRow(user("look", ["/a.png", "/b.png"])).label).toBe("◷ look  [2 images]");
});

test("the terminal's image markers are not shown or redrafted when the images come back", () => {
  const input = user("see [image 1] here [image 2]", ["/a.png", "/b.png"]);
  expect(queueRow(input).label).toBe("◷ see here  [2 images]");
  expect(redraftText([input], "")).toBe("see here");
  expect(queueRow(user("[image 1]", ["/a.png"])).label).toBe("◷  [image]");
});

test("only the user's inputs can be taken back, from any app", () => {
  expect(isUsers({ app: "term", text: "x" })).toBe(true);
  expect(isUsers(agent("x"))).toBe(false);
  expect(queueView([agent("a")], 3).editable).toBe(false);
  expect(queueView([agent("a"), { app: "term", text: "b" }], 3).editable).toBe(true);
  expect(queueRow(agent("a")).users).toBe(false);
});

test("a long queue shows one row fewer and counts the rest", () => {
  const four = ["a", "b", "c", "d"].map((text) => user(text));
  const view = queueView(four, 3);
  expect(view.rows.map((row) => row.label)).toEqual(["◷ a", "◷ b"]);
  expect(view.more).toBe(2);
  const three = queueView(four.slice(0, 3), 3);
  expect(three.rows).toHaveLength(3);
  expect(three.more).toBe(0);
  expect(queueView([], 3)).toEqual({ rows: [], more: 0, editable: false });
});

test("taken-back messages go into the draft one per paragraph, before what was typed", () => {
  const inputs = [user("first\nline two"), user("second")];
  expect(redraftText(inputs, "")).toBe("first\nline two\n\nsecond");
  expect(redraftText(inputs, "  typed meanwhile \n")).toBe(
    "first\nline two\n\nsecond\n\ntyped meanwhile",
  );
  expect(redraftText([], "typed")).toBe("typed");
});

test("the images of taken-back messages come along in order", () => {
  expect(
    redraftImages([user("a", ["/1.png"]), user("b"), user("c", ["/2.png", "/3.png"])]),
  ).toEqual(["/1.png", "/2.png", "/3.png"]);
});

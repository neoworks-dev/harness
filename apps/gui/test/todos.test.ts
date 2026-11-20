import { expect, test } from "bun:test";
import { splitTodos, toggleArgs } from "../ui/plugins/todos/todos.ts";

const items = [
  { id: "t1", text: "a", done: true },
  { id: "t2", text: "b", done: false },
  { id: "t3", text: "c", done: false },
];

test("the open todos come first, in their order, the finished ones apart", () => {
  const { open, done } = splitTodos(items);
  expect(open.map((item) => item.id)).toEqual(["t2", "t3"]);
  expect(done.map((item) => item.id)).toEqual(["t1"]);
});

test("pressing a todo checks it off, or opens it again", () => {
  expect(toggleArgs(items[1]!)).toEqual({ action: "done", id: "t2" });
  expect(toggleArgs(items[0]!)).toEqual({ action: "undone", id: "t1" });
});

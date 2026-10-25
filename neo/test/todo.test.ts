import { expect, test } from "bun:test";
import { createTodoWriteTool, parseTodos } from "../src/tools/todo.ts";
import { resultText } from "./helpers.ts";

test("todo_write summarizes the list and returns it as details", async () => {
  const tool = createTodoWriteTool();
  const todos = [{ content: "only one", status: "in_progress" as const }];
  const result = await tool.execute("t", { todos });
  expect(resultText(result)).toBe("1 todo: 0 done, 1 in progress, 0 pending");
  expect(result.details).toEqual({ todos });
  expect(resultText(await tool.execute("t", { todos: [] }))).toBe(
    "0 todos: 0 done, 0 in progress, 0 pending",
  );
});

test("parseTodos accepts valid lists and rejects anything else", () => {
  expect(parseTodos({ todos: [{ content: "a", status: "pending", extra: 1 }] })).toEqual([
    { content: "a", status: "pending" },
  ]);
  expect(parseTodos({ todos: [{ content: "a", status: "stuck" }] })).toBeUndefined();
  expect(parseTodos({ todos: [{ status: "pending" }] })).toBeUndefined();
  expect(parseTodos({ todos: "nope" })).toBeUndefined();
  expect(parseTodos(undefined)).toBeUndefined();
});

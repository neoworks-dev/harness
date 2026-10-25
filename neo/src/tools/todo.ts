import type { AgentTool } from "../loop/index.ts";
import type { PlanEntry, PlanEntryStatus } from "@neoworks-dev/neo-protocol/types";
import { Type } from "typebox";
import { isRecord } from "../util.ts";

const parameters = Type.Object({
  todos: Type.Array(
    Type.Object({
      content: Type.String({ description: "What to do, as a short imperative." }),
      status: Type.Union([
        Type.Literal("pending"),
        Type.Literal("in_progress"),
        Type.Literal("completed"),
      ]),
    }),
    { description: "The complete todo list, in order. It replaces the previous list." },
  ),
});

export interface TodoDetails {
  todos: PlanEntry[];
}

const STATUSES: PlanEntryStatus[] = ["pending", "in_progress", "completed"];

/** The todos in a todo_write call's arguments; undefined when they don't have the right shape. */
export function parseTodos(args: unknown): PlanEntry[] | undefined {
  if (!isRecord(args) || !Array.isArray(args.todos)) return undefined;
  const todos: PlanEntry[] = [];
  for (const item of args.todos) {
    if (!isRecord(item) || typeof item.content !== "string") return undefined;
    const status = STATUSES.find((candidate) => candidate === item.status);
    if (!status) return undefined;
    todos.push({ content: item.content, status });
  }
  return todos;
}

function summarize(todos: PlanEntry[]): string {
  const count = (status: PlanEntryStatus): number =>
    todos.filter((todo) => todo.status === status).length;
  const noun = todos.length === 1 ? "todo" : "todos";
  return (
    `${todos.length} ${noun}: ${count("completed")} done, ` +
    `${count("in_progress")} in progress, ${count("pending")} pending`
  );
}

export function createTodoWriteTool(): AgentTool<typeof parameters, TodoDetails> {
  return {
    name: "todo_write",
    label: "todo_write",
    description:
      "Keep a todo list for the current task. Use it for work with several steps, and update " +
      "it as you go: send the complete list every time, it replaces the previous one. Keep " +
      "exactly one todo in_progress while working, mark each one completed as soon as it is " +
      "done, and don't use it for simple one-step requests.",
    parameters,
    execute(_toolCallId, { todos }) {
      return Promise.resolve({
        content: [{ type: "text", text: summarize(todos) }],
        details: { todos },
      });
    },
  };
}

// The session's shared todo list: the user (from any app, through the neo.todos action) and
// every agent (through the todos core tool) read and change the same list. Each change appends a
// `todos` event with the whole list after it, so the list replays with the session and every
// app follows it; the newest event is the current list. Items keep their id (t1, t2, …) for
// their life, so a change names what it changes even after others moved things.
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { TodoItem } from "@neoworks-dev/neo-protocol/types";
import type { SessionStore } from "./store.ts";

/** One change to the list. */
export type TodoChange =
  | { op: "add"; text: string }
  | { op: "edit"; id: string; text: string }
  | { op: "done" | "undone" | "remove"; id: string };

export class Todos {
  constructor(private readonly store: SessionStore) {}

  /** The current list: the newest todos event's. */
  list(session: string): TodoItem[] {
    const events = this.store.readTypes(session, ["todos"]);
    const last = events.at(-1)?.event;
    if (last?.type !== "todos") return [];
    return last.items;
  }

  /** Makes the change as `by` (`user` or an agent's name); the list after it, or why not. */
  change(session: string, change: TodoChange, by: string): TodoItem[] | string {
    const items = this.list(session).map((item) => ({ ...item }));
    if (change.op === "add") {
      const text = change.text.trim();
      if (text === "") return "a todo needs text";
      items.push({ id: nextId(items), text, done: false, by });
    } else {
      const at = items.findIndex((item) => item.id === change.id);
      const item = items[at];
      if (item === undefined) return `there is no todo ${change.id}`;
      if (change.op === "remove") items.splice(at, 1);
      else if (change.op === "edit") {
        const text = change.text.trim();
        if (text === "") return "a todo needs text";
        item.text = text;
      } else item.done = change.op === "done";
    }
    this.store.append(session, { type: "todos", items, by });
    return items;
  }
}

/** `t` and one more than the highest number in use. */
function nextId(items: TodoItem[]): string {
  const numbers = items.map((item) => Number(/^t(\d+)$/.exec(item.id)?.[1] ?? 0));
  return `t${Math.max(0, ...numbers) + 1}`;
}

/** The list as the todos tool returns it: `t3 [x] Fix the header (user)`, one per line. */
export function todosText(items: TodoItem[]): string {
  if (items.length === 0) return "The todo list is empty.";
  return items
    .map((item) => {
      const line = `${item.id} [${item.done ? "x" : " "}] ${item.text}`;
      if (item.by === undefined) return line;
      return `${line} (${item.by})`;
    })
    .join("\n");
}

/** The todos tool's (or the neo.todos action's) arguments as a change; undefined for list. */
export function parseTodoArgs(args: Record<string, unknown>): TodoChange | undefined | string {
  const action = typeof args.action === "string" ? args.action : "list";
  const text = typeof args.text === "string" ? args.text : undefined;
  const id = typeof args.id === "string" ? args.id : undefined;
  switch (action) {
    case "list":
      return undefined;
    case "add":
      return text === undefined ? "todos: add needs text" : { op: "add", text };
    case "edit":
      return id === undefined || text === undefined
        ? "todos: edit needs id and text"
        : { op: "edit", id, text };
    case "done":
    case "undone":
    case "remove":
      return id === undefined ? `todos: ${action} needs id` : { op: action, id };
    default:
      return "todos: action is one of list, add, edit, done, undone, remove";
  }
}

const DID: Record<TodoChange["op"], string> = {
  add: "Added",
  edit: "Changed",
  done: "Checked off",
  undone: "Opened again",
  remove: "Removed",
};

/**
 * Runs the todos tool for `by` in `session`: for list the whole list; for a change what it did
 * to which item and how many are open and done, so a call reads as one line; or why not.
 */
export function todosTool(
  todos: Todos,
  session: string,
  by: string,
  args: Record<string, unknown>,
): CallToolResult {
  const fail = (text: string): CallToolResult => ({
    content: [{ type: "text", text }],
    isError: true,
  });
  const change = parseTodoArgs(args);
  if (typeof change === "string") return fail(change);
  if (change === undefined)
    return { content: [{ type: "text", text: todosText(todos.list(session)) }] };
  const before = todos.list(session);
  const items = todos.change(session, change, by);
  if (typeof items === "string") return fail(`todos: ${items}`);
  const item =
    change.op === "add"
      ? items.at(-1)
      : (items.find((i) => i.id === change.id) ?? before.find((i) => i.id === change.id));
  const done = items.filter((i) => i.done).length;
  const counts = `${items.length - done} open, ${done} done.`;
  const what = item === undefined ? "" : `: ${todosText([item])}`;
  return { content: [{ type: "text", text: `${DID[change.op]}${what}\n${counts}` }] };
}

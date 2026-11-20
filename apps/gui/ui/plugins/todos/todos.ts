// The session's shared todo list as its `todos` events leave it: each carries the whole list after
// one change, so the newest is current. Plain functions, tested without a window.
import type { TodoItem } from "@neoworks-dev/neo-protocol/types";

export const NO_TODOS: readonly TodoItem[] = [];

/** The open items first, in their order, then the finished ones. */
export function splitTodos(items: readonly TodoItem[]): { open: TodoItem[]; done: TodoItem[] } {
  return { open: items.filter((item) => !item.done), done: items.filter((item) => item.done) };
}

/** What `neo.todos` is asked to do when an item is pressed: check it off, or open it again. */
export function toggleArgs(item: TodoItem): { action: "done" | "undone"; id: string } {
  return { action: item.done ? "undone" : "done", id: item.id };
}

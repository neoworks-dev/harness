// The session's shared todo list, at the top of the column at the right (above the plan), as the
// terminal app has it.
import type { Context } from "@neoworks/extension-system";
import "../sidepanel/slots.ts";
import { todos } from "./state.svelte.ts";
import Todos from "./Todos.svelte";

export const todosPanel = {
  name: "todos",
  inject: ["slots", "daemon", "sidepanel"],
  apply(ctx: Context) {
    const { slots, daemon, sidepanel } = ctx;
    todos.follow(daemon);

    slots.inject("sidepanel.section", (slot) => {
      slot.add({ component: Todos, order: 5 });
    });
    sidepanel.claim(() => todos.items.length > 0);
  },
};

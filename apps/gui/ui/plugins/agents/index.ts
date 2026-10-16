// The agents working for the session, under the input: one row per subagent or guest. Ctrl-X
// gives the list the keyboard. Enter on a row opens that agent's own chat (the agentview plugin).
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import Agents from "./Agents.svelte";
import { agents } from "./state.svelte.ts";

export const agentsList = {
  name: "agents",
  inject: ["slots", "daemon", "keymap", "keyboard", "agentview"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, keyboard, agentview } = ctx;
    agents.follow(daemon);

    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: Agents, order: 20 });
    });

    // The input leaves the keyboard alone while the list has it.
    keyboard.claim(() => agents.focused);
    // An agent's own chat has the keyboard itself, and its Esc comes back.
    keymap.bind("ctrl-x", () => {
      if (agentview.isOpen) return false;
      agents.focused = !agents.focused;
    });
  },
};

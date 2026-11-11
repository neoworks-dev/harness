// The terminal: the commands the agent and its guests ran and what they printed, in a panel beside
// the conversation (Ctrl-`, /terminal, or its button at the right). It takes the side panel's
// place while it is open, as the viewer does.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import SidePane from "./SidePane.svelte";
import { terminal } from "./state.svelte.ts";
import ToolbarButton from "./ToolbarButton.svelte";

export const terminalPane = {
  name: "terminal",
  // daemon: the panel reads the session's items through useCtx.
  inject: ["slots", "daemon", "keymap", "commands", "sidepanel"],
  apply(ctx: Context) {
    const { slots, keymap, commands, sidepanel } = ctx;

    slots.inject("shell.right", (slot) => {
      slot.add({ component: SidePane, order: -5 });
    });
    slots.inject("shell.toolbar", (slot) => {
      slot.add({ component: ToolbarButton, order: 17 });
    });
    sidepanel.cover(() => terminal.shown);

    const toggle = (): void => terminal.toggle();
    keymap.bind("ctrl-`", toggle);
    commands.register({
      name: "terminal",
      desc: "the agent's commands and their output beside the chat (ctrl-`)",
      run: toggle,
    });
  },
};

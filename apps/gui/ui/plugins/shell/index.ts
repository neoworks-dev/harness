// The shell owns the root slot and declares the slots the window's panes register into. It is
// the only plugin that knows how they are laid out.
import type { Context } from "@neoworks/extension-system";
import Shell from "./Shell.svelte";
import "./slots.ts";

export const shell = {
  name: "shell",
  inject: ["slots", "daemon"],
  apply({ slots }: Context) {
    slots.add("root", {
      component: Shell,
      slots: {
        "shell.left": "list",
        "shell.top": "list",
        "shell.top.right": "list",
        "shell.toolbar": "list",
        "shell.main": "chain",
        "shell.bottom": "list",
        "shell.right": "list",
        "shell.card": "list",
        "shell.overlay": "list",
      },
    });
  },
};

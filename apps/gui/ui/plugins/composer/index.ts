// The input under the conversation: a draft with history and a `/` menu, the panel that answers
// the agent's questions, `/help`, and the keys around them (Esc, Ctrl-C, Ctrl-P, Shift-Tab). It
// declares `composer.left` and `composer.right` in the row of the input, and fills the status
// line's `statusline.right` with its short messages.
import type { Context } from "@neoworks/extension-system";
import { window } from "@neoworks/svelte-gpui";
import { chord } from "../../kernel/keymap.ts";
import "../shell/slots.ts";
import "../statusline/slots.ts";
import Composer from "./Composer.svelte";
import Notice from "./Notice.svelte";
import "./slots.ts";
import { composer } from "./state.svelte.ts";

export const composerPane = {
  name: "composer",
  inject: ["slots", "daemon", "keymap", "keyboard", "commands", "transcript", "diff", "comments"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, commands, comments } = ctx;
    ctx.effect(() => composer.start({ daemon, commands, comments }), "composer state");

    slots.inject("shell.bottom", (slot) => {
      slot.add({
        component: Composer,
        order: 10,
        slots: { "composer.left": "list", "composer.right": "list" },
      });
    });
    slots.inject("statusline.right", (slot) => {
      slot.add({ component: Notice });
    });

    // Ctrl-C is left to the input too, so copying a selection still works; any other key
    // disarms a first Ctrl-C.
    keymap.observe((event) => {
      if (!/^ctrl-(shift-)?c$/.test(chord(event) ?? "")) composer.otherKey();
    });
    for (const copy of ["ctrl-c", "ctrl-shift-c"]) {
      keymap.bind(copy, () => {
        composer.ctrlC();
        return false;
      });
    }

    keymap.bind("shift-tab", () => void daemon.cycleApprovals());
    keymap.bind("escape", (event) => {
      // Something that took Escape itself (a popover) has already closed.
      if (event.defaultPrevented) return false;
      if (composer.escape()) return true;
      if (!daemon.status.running) return false;
      void daemon.cancel();
    });
    keymap.bind("ctrl-p", () => composer.reopenPrompts());

    commands.register({
      name: "help",
      desc: "keys and commands",
      run: () => (composer.helpOpen = true),
    });
    commands.register({
      name: "quit",
      desc: "close the window; the session keeps running",
      run: () => window.close(),
    });
  },
};

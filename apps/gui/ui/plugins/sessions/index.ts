// The sessions sidebar: every session grouped by repository and worktree, in the shell's left
// column. It owns its state (state.svelte.ts), its keys and its `/sidebar` command; a plugin that
// wants to know whether it is shown reads `sessions.open` there.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import Sessions from "./Sessions.svelte";
import { sessions } from "./state.svelte.ts";

export const sessionsSidebar = {
  name: "sessions",
  inject: ["slots", "daemon", "keymap", "commands"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, commands } = ctx;
    // State that runs its own effects is started through `ctx.effect`, so unloading stops it.
    ctx.effect(() => sessions.start(daemon), "sessions state");

    slots.inject("shell.left", (slot) => {
      slot.add({ component: Sessions });
    });

    const toggle = (): void => {
      sessions.open = !sessions.open;
    };
    keymap.bind("ctrl-s", toggle);
    // F6 opens the sidebar and gives it the keyboard.
    keymap.bind("f6", () => {
      sessions.open = true;
      sessions.focused = !sessions.focused;
    });
    keymap.bind("ctrl-n", () => void daemon.newSession());
    commands.register({
      name: "sidebar",
      desc: "show or hide the sessions sidebar (ctrl-s)",
      run: toggle,
    });
  },
};

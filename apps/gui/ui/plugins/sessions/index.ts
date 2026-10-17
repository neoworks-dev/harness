// The sessions sidebar: every session grouped by repository and worktree, in the shell's left
// column. It owns its state (state.svelte.ts), its keys and its `/sidebar` command; a plugin that
// wants to know whether it is shown reads `sessions.open` there.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import Breadcrumb from "./Breadcrumb.svelte";
import Sessions from "./Sessions.svelte";
import SidebarToggle from "./SidebarToggle.svelte";
import { sessions } from "./state.svelte.ts";

export const sessionsSidebar = {
  name: "sessions",
  inject: ["slots", "daemon", "keymap", "keyboard", "commands"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, keyboard, commands } = ctx;
    // State that runs its own effects is started through `ctx.effect`, so unloading stops it.
    ctx.effect(() => sessions.start(daemon), "sessions state");

    slots.inject("shell.left", (slot) => {
      slot.add({ component: Sessions });
    });
    slots.inject("shell.top", (slot) => {
      slot.add({ component: SidebarToggle, order: 0 });
      slot.add({ component: Breadcrumb, order: 10 });
    });

    // The input leaves the keyboard alone while the sidebar has it.
    keyboard.claim(() => sessions.focused);

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
    // Ctrl-F is left free for finding in a file; this one searches the sessions.
    keymap.bind("ctrl-shift-f", () => sessions.focusSearch());
    commands.register({
      name: "sidebar",
      desc: "show or hide the sessions sidebar (ctrl-s)",
      run: toggle,
    });
  },
};

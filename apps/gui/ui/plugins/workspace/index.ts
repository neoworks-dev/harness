// The workspace card (Ctrl-G, `/workspace`): a floating card at the conversation's top right with
// the session's folder, the editor to open it in, the branch, a commit button and the changes.
// It fills the shell's `shell.card` slot and the toolbar strip, and owns its toggle and key. The
// host does the work (apps/gui/src/desktop.rs, workspace.rs); the diff of the changes is shown by
// the `diff` service.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import ToolbarButton from "./ToolbarButton.svelte";
import { workspace } from "./state.svelte.ts";
import Workspace from "./Workspace.svelte";

export const workspaceCard = {
  name: "workspace",
  inject: ["slots", "daemon", "diff", "keymap", "keyboard", "commands"],
  apply(ctx: Context) {
    const { slots, daemon, diff, keymap, keyboard, commands } = ctx;
    ctx.effect(() => workspace.start({ daemon, diff }), "workspace state");

    slots.inject("shell.card", (slot) => {
      slot.add({ component: Workspace });
    });
    slots.inject("shell.toolbar", (slot) => {
      slot.add({ component: ToolbarButton, order: 30 });
    });

    // The agent edited files or finished a turn: the counts are worth asking for again.
    daemon.onRecord((record) => {
      const { type } = record.event;
      if (type === "tool_call_update" || type === "turn_ended") workspace.poke();
    });

    // The input leaves the keyboard alone while the commit message is being typed.
    keyboard.claim(() => workspace.committing !== undefined);

    const toggle = (): void => {
      workspace.hidden = !workspace.hidden;
    };
    keymap.bind("ctrl-g", toggle);
    commands.register({
      name: "workspace",
      desc: "show or hide the workspace card (ctrl-g)",
      run: toggle,
    });
  },
};

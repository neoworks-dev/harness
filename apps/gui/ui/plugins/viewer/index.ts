// The viewer: a panel beside the conversation (or in its place, maximized) with a diff of the
// session's changes and the files opened from it. Lines of either can be selected and commented;
// the comments go to the input through the `comments` service. Other panes open it through the
// `viewer` service; it replaces the full-window diff.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import MaxPane from "./MaxPane.svelte";
import { ViewerService } from "./service.ts";
import SidePane from "./SidePane.svelte";
import { viewer } from "./state.svelte.ts";
import ToolbarButton from "./ToolbarButton.svelte";

export const viewerPane = {
  name: "viewer",
  inject: ["slots", "daemon", "git", "keymap", "keyboard", "commands", "sidepanel", "comments"],
  apply(ctx: Context) {
    const { slots, daemon, git, keymap, keyboard, commands, sidepanel, comments } = ctx;
    ctx.effect(() => viewer.start({ daemon, git, comments }), "viewer state");
    viewer.follow(daemon);
    ctx.plugin(ViewerService);

    // Beside the conversation, ahead of the side panel; maximized, in the conversation's place.
    slots.inject("shell.right", (slot) => {
      slot.add({ component: SidePane, order: -10 });
    });
    slots.inject("shell.main", (slot) => {
      slot.add({ component: MaxPane, order: 5, select: () => viewer.shown && viewer.maximized });
    });
    slots.inject("shell.toolbar", (slot) => {
      slot.add({ component: ToolbarButton, order: 15 });
    });

    // It takes the side panel's place while it is open, and the keyboard while a comment is written.
    sidepanel.cover(() => viewer.shown && !viewer.maximized);
    keyboard.claim(() => viewer.editing);

    // Esc puts a selection away before it does anything else.
    keymap.bind("escape", () => {
      if (viewer.selection === undefined) return false;
      viewer.clearSelection();
    });

    const toggle = (): void => viewer.toggle();
    keymap.bind("ctrl-d", toggle);
    commands.register({
      name: "diff",
      desc: "the diff and files beside the chat (ctrl-d)",
      run: toggle,
    });
  },
};

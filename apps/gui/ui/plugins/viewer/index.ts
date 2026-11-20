// The viewer: a panel beside the conversation (or in its place, maximized) with a diff of the
// session's changes and the files opened from it. Lines of either can be selected and commented;
// the comments go to the input through the `comments` service. Other panes open it through the
// `viewer` service; it replaces the full-window diff.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import MaxPane from "./MaxPane.svelte";
import { permit } from "./permit.svelte.ts";
import PermitPane from "./PermitPane.svelte";
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
      // An edit waiting for its answer comes before everything else in the chat's place.
      slot.add({ component: PermitPane, order: 1, select: () => permit.current !== undefined });
      slot.add({ component: MaxPane, order: 5, select: () => viewer.shown && viewer.maximized });
    });
    permit.follow(daemon);
    keyboard.claim(() => permit.current !== undefined);
    slots.inject("shell.toolbar", (slot) => {
      slot.add({ component: ToolbarButton, order: 15 });
    });

    // It takes the side panel's place while it is open, and the keyboard while a comment is written
    // or a file in the conversation's place is reviewed with the keys (MaxPane).
    sidepanel.cover(() => viewer.shown && !viewer.maximized);
    keyboard.claim(() => viewer.editing || viewer.reviewing);

    // Ctrl-E: the latest turn's changed files, one after another, each whole in the chat's place.
    // Edits put aside with Esc come back first.
    keymap.bind("ctrl-e", () => {
      if (permit.bringBack()) return;
      if (!viewer.reviewTurn(0)) daemon.error = "The latest turn changed no files.";
    });

    // Esc puts a selection away before it does anything else.
    keymap.bind("escape", () => {
      if (viewer.selection !== undefined) {
        viewer.clearSelection();
        return;
      }
      // A file opened from the chat: back to the conversation.
      if (!viewer.inPlace) return false;
      viewer.close();
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

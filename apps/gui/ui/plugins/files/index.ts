// The files panel, in the column at the right: what git says changed in the session's directory,
// plus the files the agent and its guests touched. Pressing a file shows its diff over the
// window (the diff plugin).
import type { Context } from "@neoworks/extension-system";
import "../sidepanel/slots.ts";
import Files from "./Files.svelte";
import { fileEntries } from "./files.ts";
import { git } from "./git.svelte.ts";
import { touched } from "./state.svelte.ts";

export const filesPanel = {
  name: "files",
  inject: ["slots", "daemon", "git", "sidepanel", "diff"],
  apply(ctx: Context) {
    const { slots, daemon, git: gitService, sidepanel } = ctx;
    touched.follow(daemon);
    ctx.effect(() => git.start({ daemon, git: gitService }, () => touched.changes), "git status");

    slots.inject("sidepanel.section", (slot) => {
      slot.add({ component: Files, order: 20 });
    });
    sidepanel.claim(() => fileEntries(git.status, touched.touches.touched).length > 0);
  },
};

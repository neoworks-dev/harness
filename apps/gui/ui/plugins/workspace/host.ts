// The host commands the workspace card calls (apps/gui/src/desktop.rs and workspace.rs), as
// functions. They are the card's own; the files plugin's status is not reachable from here.
import { invoke } from "@neoworks/svelte-gpui";
import type { Reply } from "../../kernel/host/reply.ts";

/** This machine's name. */
export const hostname = (): Reply<"hostname"> => invoke("hostname", null);

/** The known editors found on PATH. */
export const editors = (): Reply<"editors"> => invoke("editors", null);

/** Opens a folder in the editor with this id. */
export const openInEditor = (editor: string, path: string): Reply<"open_in_editor"> =>
  invoke("open_in_editor", { editor, path });

/** Shows a folder in the system's file manager. */
export const openFolder = (path: string): Reply<"open_folder"> => invoke("open_folder", { path });

/** The branch, its upstream and what changed in `cwd`'s repository; null outside one. */
export const overview = (cwd: string): Reply<"git_workspace"> => invoke("git_workspace", { cwd });

/** The local branches, the latest commit first, and the one checked out. */
export const branches = (cwd: string): Reply<"git_branches"> => invoke("git_branches", { cwd });

/** Switches branch; rejects when the tree has changes. */
export const checkout = (cwd: string, branch: string): Reply<"git_checkout"> =>
  invoke("git_checkout", { cwd, branch });

/** Commits all changes, then pushes when asked and the branch has an upstream. */
export const commit = (cwd: string, message: string, push: boolean): Reply<"git_commit"> =>
  invoke("git_commit", { cwd, message, push });

/** The unified diff of everything changed against HEAD. */
export const diffAll = (cwd: string): Reply<"git_diff_all"> => invoke("git_diff_all", { cwd });

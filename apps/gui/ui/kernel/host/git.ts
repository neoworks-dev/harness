// The host's git commands (apps/gui/src/git.rs), as functions. The stores and the `git` service
// both call these, so the commands are named in one place.
import { invoke } from "@neoworks/svelte-gpui";
import type { Reply } from "./reply.ts";

export interface DiffArgs {
  cwd: string;
  path: string;
  /** The file is new to git: show all of it as added. */
  untracked?: boolean;
  /** The whole file with the changes marked, not only the changed hunks. */
  full?: boolean;
}

/** The branch checked out in `cwd`, or null outside a repository. */
export const branch = (cwd: string): Reply<"git_branch"> => invoke("git_branch", { cwd });

/** Where each directory works (repository and worktree), read from the `.git` files. */
export const places = (cwds: string[]): Reply<"places"> => invoke("places", { cwds });

/** `git status --porcelain=v2 --branch -z` of `cwd`; null outside a repository. */
export const status = (cwd: string): Reply<"git_status"> => invoke("git_status", { cwd });

/** The unified diff of one file against HEAD. */
export const diff = (args: DiffArgs): Reply<"git_diff"> => invoke("git_diff", args);

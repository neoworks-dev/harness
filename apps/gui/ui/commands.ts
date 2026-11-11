// What the Rust side (apps/gui/src/commands.rs) offers, written down once so `invoke` checks
// names, arguments and results.
import type { DaemonAddress } from "@neoworks-dev/neo-sdk";
import type { Candidate } from "./lib/mention.ts";
import type { Chunk } from "./plugins/terminal/jobs.ts";
import type { Detail, Summary } from "./plugins/github/github.ts";
import type { Place } from "./plugins/sessions/sessions.ts";
import type { Committed, Editor, Overview } from "./plugins/workspace/workspace.ts";

export interface Startup {
  /** Where the window was started. */
  cwd: string;
  /** `neo gui --session ID`. */
  session: string | null;
  /** `neo gui --new`. */
  new: boolean;
}

export interface GitStatusOutput {
  out: string;
  prefix: string;
}

export interface GhListArgs {
  cwd: string;
  /** `owner/name`, from `gh_repo`. */
  repo: string;
  /** Closed ones too. */
  all?: boolean;
  /** GitHub search syntax. */
  query?: string;
}

export interface GhItemArgs {
  cwd: string;
  repo: string;
  number: number;
}

declare module "@neoworks/svelte-gpui" {
  interface CommandMap {
    startup: { args: null; result: Startup };
    /** Rejects with the code `NotRunning` when there is no daemon. */
    daemon_address: { args: null; result: DaemonAddress };
    git_branch: { args: { cwd: string }; result: string | null };
    /** What an `@query` could be in `cwd` (crates/mention); with `folders`, folders only. */
    mentions: { args: { cwd: string; query: string; folders?: boolean }; result: Candidate[] };
    /** A background command's output file read on from `offset` (crates/jobs); only `….output` files. */
    job_read: { args: { path: string; offset: number }; result: Chunk };
    /** Stops the command writing that output file; how many process groups were signalled. */
    job_stop: { args: { path: string }; result: number };
    /** Where each directory works (repository and worktree), found from the `.git` files. */
    places: { args: { cwds: string[] }; result: Record<string, Place> };
    /** `git status --porcelain=v2 --branch -z` and the directory's place in the repository; null outside one. */
    git_status: { args: { cwd: string; all?: boolean }; result: GitStatusOutput | null };
    /** The unified diff of one file against HEAD, or the whole untracked file as added. */
    git_diff: {
      args: { cwd: string; path: string; untracked?: boolean; full?: boolean };
      result: string;
    };
    // The workspace card: where the directory stands in git, the little it changes, the editors
    // and the file manager. Failures carry git's own words, shortened.
    /** This machine's name. */
    hostname: { args: null; result: string };
    /** The clipboard's image saved to a file, or null when it holds none. */
    clipboard_image: { args: null; result: string | null };
    /** The known editors found on PATH. */
    editors: { args: null; result: Editor[] };
    open_in_editor: { args: { editor: string; path: string }; result: null };
    /** Shows a folder in the system's file manager. */
    open_folder: { args: { path: string }; result: null };
    /** Opens an http, https or mailto address in the system's browser or mail app; refuses others. */
    open_url: { args: { url: string }; result: null };
    /** The branch, its upstream and what changed in the directory's repository; null outside one. */
    git_workspace: { args: { cwd: string }; result: Overview | null };
    /** The local branches, the latest commit first, and the one checked out. */
    git_branches: {
      args: { cwd: string };
      result: { branches: string[]; current: string | null };
    };
    /** Switches branch; refuses when the tree has changes. */
    git_checkout: { args: { cwd: string; branch: string }; result: null };
    /** Commits all changes, then pushes when asked and there is an upstream. */
    git_commit: { args: { cwd: string; message: string; push: boolean }; result: Committed };
    /** The unified diff of everything changed against HEAD, new files included. */
    git_diff_all: { args: { cwd: string }; result: string };
    // The GitHub view. Read-only: the host runs `gh` through an allowlist and refuses anything
    // else. Failures carry the code GhMissing, NotRepo, NotAuth or GhFailed.
    gh_repo: { args: { cwd: string }; result: string };
    gh_issues: { args: GhListArgs; result: Summary[] };
    gh_prs: { args: GhListArgs; result: Summary[] };
    gh_issue: { args: GhItemArgs; result: Detail };
    gh_pr: { args: GhItemArgs; result: Detail };
    gh_pr_diff: { args: GhItemArgs; result: string };
    /** What "send to the model" puts into the session. */
    gh_issue_text: { args: GhItemArgs; result: string };
    gh_pr_text: { args: GhItemArgs; result: string };
  }
}

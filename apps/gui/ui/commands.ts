// What the Rust side (apps/gui/src/commands.rs) offers, written down once so `invoke` checks
// names, arguments and results.
import type { DaemonAddress } from "@suite/sdk";
import type { Detail, Summary } from "./lib/github.ts";
import type { Place } from "./lib/sessions.ts";

export interface Startup {
  /** Where the window was started. */
  cwd: string;
  /** `suite gui --session ID`. */
  session: string | null;
  /** `suite gui --new`. */
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
    /** Where each directory works (repository and worktree), found from the `.git` files. */
    places: { args: { cwds: string[] }; result: Record<string, Place> };
    /** `git status --porcelain=v2 --branch -z` and the directory's place in the repository; null outside one. */
    git_status: { args: { cwd: string }; result: GitStatusOutput | null };
    /** The unified diff of one file against HEAD, or the whole untracked file as added. */
    git_diff: {
      args: { cwd: string; path: string; untracked?: boolean; full?: boolean };
      result: string;
    };
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

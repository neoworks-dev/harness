// The host's GitHub commands (apps/gui/src/github.rs), as functions. Reads, and the few writes
// the view's buttons ask for; the host runs `gh` through an allowlist for both. Failures carry the
// code GhMissing, NotRepo, NotAuth or GhFailed.
import { invoke } from "@neoworks/svelte-gpui";
import type { Reply } from "./reply.ts";
import type { GhItemArgs, GhListArgs, GhWriteArgs } from "../../commands.ts";

/** `owner/name` of the repository `gh` finds for `cwd`. */
export const repo = (cwd: string): Reply<"gh_repo"> => invoke("gh_repo", { cwd });
export const issues = (args: GhListArgs): Reply<"gh_issues"> => invoke("gh_issues", args);
export const prs = (args: GhListArgs): Reply<"gh_prs"> => invoke("gh_prs", args);
export const issue = (args: GhItemArgs): Reply<"gh_issue"> => invoke("gh_issue", args);
export const pr = (args: GhItemArgs): Reply<"gh_pr"> => invoke("gh_pr", args);
export const prDiff = (args: GhItemArgs): Reply<"gh_pr_diff"> => invoke("gh_pr_diff", args);
/** What "send to the model" puts into the session. */
export const issueText = (args: GhItemArgs): Reply<"gh_issue_text"> =>
  invoke("gh_issue_text", args);
export const prText = (args: GhItemArgs): Reply<"gh_pr_text"> => invoke("gh_pr_text", args);
/** The repository's labels with their colors. */
export const labels = (args: { cwd: string; repo: string }): Reply<"gh_labels"> =>
  invoke("gh_labels", args);
// Writes: each one button the user pressed, as the user `gh` is logged in as.
export const comment = (args: GhWriteArgs & { body: string }): Reply<"gh_comment"> =>
  invoke("gh_comment", args);
export const close = (args: GhWriteArgs & { reason?: "completed" | "not planned" }): Reply<"gh_close"> =>
  invoke("gh_close", args);
export const reopen = (args: GhWriteArgs): Reply<"gh_reopen"> => invoke("gh_reopen", args);
export const react = (args: GhItemArgs & { content: string; comment?: number }): Reply<"gh_react"> =>
  invoke("gh_react", args);

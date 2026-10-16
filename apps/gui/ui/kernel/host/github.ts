// The host's GitHub commands (apps/gui/src/github.rs), as functions. They only read: the host
// runs `gh` through an allowlist. Failures carry the code GhMissing, NotRepo, NotAuth or GhFailed.
import { invoke } from "@neoworks/svelte-gpui";
import type { GhItemArgs, GhListArgs } from "../../commands.ts";

/** `owner/name` of the repository `gh` finds for `cwd`. */
export const repo = (cwd: string) => invoke("gh_repo", { cwd });
export const issues = (args: GhListArgs) => invoke("gh_issues", args);
export const prs = (args: GhListArgs) => invoke("gh_prs", args);
export const issue = (args: GhItemArgs) => invoke("gh_issue", args);
export const pr = (args: GhItemArgs) => invoke("gh_pr", args);
export const prDiff = (args: GhItemArgs) => invoke("gh_pr_diff", args);
/** What "send to the model" puts into the session. */
export const issueText = (args: GhItemArgs) => invoke("gh_issue_text", args);
export const prText = (args: GhItemArgs) => invoke("gh_pr_text", args);

// What the workspace card works out from what the host answers: line counts, which editor the
// primary button opens, what the commit button does and says. Pure, so tests reach it.

/** An editor found on the machine's PATH. */
export interface Editor {
  id: string;
  name: string;
}

/** How the working tree changed against HEAD. */
export interface Counts {
  /** Files changed, untracked ones included. */
  files: number;
  added: number;
  deleted: number;
}

export const NO_COUNTS: Counts = { files: 0, added: 0, deleted: 0 };

/**
 * Sums `git diff --numstat` (lines of `added<TAB>deleted<TAB>path`, `-` for a binary file) and
 * the line counts of the untracked files (0 for a binary one): a file is a file either way.
 */
export function sumStat(numstat: string, untracked: readonly number[]): Counts {
  const counts = { files: untracked.length, added: 0, deleted: 0 };
  for (const line of numstat.split("\n")) {
    const [added, deleted] = line.split("\t");
    if (added === undefined || deleted === undefined) continue;
    counts.files += 1;
    counts.added += Number.parseInt(added, 10) || 0;
    counts.deleted += Number.parseInt(deleted, 10) || 0;
  }
  for (const lines of untracked) counts.added += lines;
  return counts;
}

/** 999, then 1.2k, 15k: a count that fits the card's row. */
export function shortCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  return `${Math.round(n / 1000)}k`;
}

/** The last part of a path. */
export function folderName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || path;
}

/** The editor the primary button opens: the last one used while it is still found, else the first. */
export function preferredEditor(
  editors: readonly Editor[],
  lastUsed: string | undefined,
): Editor | undefined {
  return editors.find((editor) => editor.id === lastUsed) ?? editors[0];
}

/** What the host knows of the directory's repository (`git_workspace`). */
export interface Overview {
  /** None when HEAD is detached. */
  branch: string | null;
  upstream: string | null;
  /** Commits not pushed. */
  ahead: number;
  dirty: boolean;
  numstat: string;
  untracked: number[];
}

/** What `git_commit` answers. */
export interface Committed {
  /** The new commit's short id; none when only a push was due. */
  commit: string | null;
  pushed: boolean;
  /** Why nothing was pushed, or what went wrong with the push. */
  note: string | null;
  failed: boolean;
}

/** What the primary button of the commit split does. */
export type Primary =
  | { kind: "commit-push"; label: string }
  | { kind: "push"; label: string }
  | { kind: "none"; label: string };

/**
 * Committing and pushing asks for a message when something changed; with nothing changed but
 * commits to push it only pushes; with neither it has nothing to do.
 */
export function primaryAction(overview: Overview): Primary {
  if (overview.dirty) return { kind: "commit-push", label: "Commit & push" };
  if (overview.ahead > 0 && overview.upstream !== null) return { kind: "push", label: "Push" };
  return { kind: "none", label: "Commit & push" };
}

/** The sentence under the card after a commit: what was done, then why the push was not. */
export function committedMessage(done: Committed): string {
  let text = "";
  if (done.commit !== null) text = `Committed ${done.commit}`;
  if (done.pushed) text = done.commit === null ? "Pushed" : `${text} and pushed`;
  if (done.note === null) return text;
  return text === "" ? done.note : `${text}. ${done.note}`;
}

/** What the agent is asked when the user leaves the commit to it. */
export const ASK_AGENT_TO_COMMIT =
  "Review the uncommitted changes in the working tree (git status and git diff), then commit them with a clear commit message that says what changed and why. Do not push.";

/** An error as the card shows it: one line. */
export function errorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.split("\n")[0]?.trim() || "Something went wrong";
}

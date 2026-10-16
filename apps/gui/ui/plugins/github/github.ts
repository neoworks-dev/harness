// The GitHub view's data, as the host's `gh_*` commands answer it (crates/github serialises the
// same shapes), and what the view derives from it. The view is read-only: nothing here asks
// GitHub to change anything.

export type Kind = "issue" | "pr";
export type State = "open" | "closed" | "merged" | "draft";
export type CheckState = "pass" | "fail" | "pending" | "skipped";

/** A row of a list. */
export interface Summary {
  number: number;
  title: string;
  state: State;
  author: string;
  labels: string[];
  comments: number;
  updatedMs: number;
}

export interface Comment {
  author: string;
  atMs: number;
  body: string;
}

export interface Review {
  author: string;
  atMs: number;
  /** APPROVED, CHANGES_REQUESTED, COMMENTED, DISMISSED, PENDING. */
  state: string;
  body: string;
}

/** A comment on a line of the diff. */
export interface ReviewComment {
  author: string;
  atMs: number;
  path: string;
  line: number | null;
  body: string;
  reply: boolean;
}

export interface Check {
  name: string;
  state: CheckState;
}

export interface FileChange {
  path: string;
  additions: number;
  deletions: number;
}

/** One issue or pull request with everything the detail shows. */
export interface Detail {
  kind: Kind;
  number: number;
  title: string;
  state: State;
  author: string;
  labels: string[];
  body: string;
  url: string;
  createdMs: number;
  updatedMs: number;
  comments: Comment[];
  // Pull requests only from here on.
  reviews: Review[];
  reviewComments: ReviewComment[];
  checks: Check[];
  files: FileChange[];
  base: string;
  head: string;
  decision: string;
  additions: number;
  deletions: number;
}

/** What `gh` failures are called, so the view can say what to do. */
export type GhErrorCode = "GhMissing" | "NotRepo" | "NotAuth" | "GhFailed";

/** "now", "5m", "3h", "2d", "3w", "5mo", "2y": the compact age of a unix time in ms. */
export function shortAge(thenMs: number, nowMs: number): string {
  if (thenMs <= 0) return "";
  const secs = Math.floor(Math.max(0, nowMs - thenMs) / 1000);
  if (secs < 60) return "now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h`;
  if (secs < 604_800) return `${Math.floor(secs / 86_400)}d`;
  if (secs < 2_592_000) return `${Math.floor(secs / 604_800)}w`;
  if (secs < 31_536_000) return `${Math.floor(secs / 2_592_000)}mo`;
  return `${Math.floor(secs / 31_536_000)}y`;
}

/** The checks counted by outcome. */
export function checkCounts(checks: readonly Check[]): Record<CheckState, number> {
  const counts: Record<CheckState, number> = { pass: 0, fail: 0, pending: 0, skipped: 0 };
  for (const check of checks) counts[check.state] += 1;
  return counts;
}

/** An entry of the conversation: a comment, a review or a comment on a line of the diff. */
export interface TimelineEntry {
  author: string;
  atMs: number;
  /** "review approved", "on src/a.ts:12", or empty for a plain comment. */
  what: string;
  body: string;
}

/** Comments, reviews and line comments in time order. Reviews without words are left out. */
export function timeline(detail: Detail): TimelineEntry[] {
  const entries: TimelineEntry[] = detail.comments.map((c) => ({ ...c, what: "" }));
  for (const review of detail.reviews) {
    if (review.state.toUpperCase() === "COMMENTED" && review.body.trim() === "") continue;
    entries.push({
      author: review.author,
      atMs: review.atMs,
      what: `review ${review.state.toLowerCase().replaceAll("_", " ")}`,
      body: review.body,
    });
  }
  for (const c of detail.reviewComments) {
    const place = c.line === null ? `on ${c.path}` : `on ${c.path}:${c.line}`;
    entries.push({
      author: c.author,
      atMs: c.atMs,
      what: c.reply ? `reply ${place}` : place,
      body: c.body,
    });
  }
  return entries.sort((a, b) => a.atMs - b.atMs);
}

/** Markdown shown as text: without the HTML comments issue templates are full of, and without `\r`. */
export function readable(markdown: string): string {
  return markdown
    .replace(/<!--[\s\S]*?(?:-->|$)/g, "")
    .replaceAll("\r", "")
    .trim();
}

/** The one-line facts of a list row: `#12 · alice · bug, ui · 3 comments`. */
export function rowFacts(row: Summary): string {
  const parts = [`#${row.number}`, row.author];
  if (row.labels.length > 0) parts.push(row.labels.join(", "));
  if (row.comments > 0) parts.push(row.comments === 1 ? "1 comment" : `${row.comments} comments`);
  return parts.join(" · ");
}

/** What the view says when `gh` can't answer. */
export function explain(code: string | undefined, message: string): string {
  switch (code) {
    case "GhMissing":
      return "The GitHub CLI (gh) is not installed. See https://cli.github.com";
    case "NotRepo":
      return "This directory is not a GitHub repository (no git remote that gh knows).";
    case "NotAuth":
      return "gh is not logged in. Run `gh auth login` in a terminal.";
    default:
      return message;
  }
}

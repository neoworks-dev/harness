// An edit the agent asks to make (or, in review mode, made and asks to keep), reviewed whole in
// the chat's place: the requests that wait, folded from the session's events; the lines the
// editor walks; the comments written on them and the note they make for the agent. Plain
// functions, tested without a window.
import type {
  DiffLine,
  PermissionOption,
  SessionEventRecord,
} from "@neoworks-dev/neo-protocol/types";
import { formatMessage, type CodeComment } from "../../lib/comments.ts";
import { relativize } from "../../lib/git-status.ts";
import { lineDiff } from "./linediff.ts";
import { quoteDiff } from "./selection.ts";

/** One file of an edit: its path in the session's directory and its lines, old and new merged. */
export interface EditFile {
  path: string;
  lines: readonly DiffLine[];
}

/** A permission request about a change to files, waiting for its answer. */
export interface PendingEdit {
  id: string;
  /** The chat item that asks it. */
  ref: string;
  title: string;
  files: readonly EditFile[];
  options: readonly PermissionOption[];
}

/** The edit requests that wait, oldest first, after one more event. */
export function applyPermit(
  pending: readonly PendingEdit[],
  record: SessionEventRecord,
  cwd: string,
): readonly PendingEdit[] {
  const { event } = record;
  if (event.type === "permission_request") {
    const files = (event.content ?? []).flatMap((content) =>
      content.type === "diff"
        ? [{ path: relativize(cwd, content.path), lines: lineDiff(content.oldText, content.newText) }]
        : [],
    );
    if (files.length === 0) return pending;
    const edit = { id: event.id, ref: `p-${event.id}`, title: event.title, files, options: event.options };
    return [...pending.filter((other) => other.id !== event.id), edit];
  }
  if (event.type === "permission_resolved") {
    return pending.some((edit) => edit.id === event.id)
      ? pending.filter((edit) => edit.id !== event.id)
      : pending;
  }
  return pending;
}

/** The option Enter picks (allow, keep) and the one that declines (reject, revert). */
export function answers(options: readonly PermissionOption[]): {
  accept?: PermissionOption;
  decline?: PermissionOption;
} {
  const first = (prefix: string, once: string) =>
    options.find((option) => option.kind === once) ??
    options.find((option) => option.kind.startsWith(prefix));
  return { accept: first("allow", "allow_once"), decline: first("reject", "reject_once") };
}

/** A line of the edit as the cursor walks it: which file, and which of its lines. */
export interface Spot {
  file: number;
  index: number;
}

/** Every line of the edit in order, the files one after the other. */
export function spotsOf(files: readonly EditFile[]): Spot[] {
  return files.flatMap((file, at) => file.lines.map((_, index) => ({ file: at, index })));
}

/**
 * The spot (in `spotsOf`'s order) where the next run of changed lines after `from` starts, or the
 * one before it (`direction` -1); undefined when there is none that way.
 */
export function nextChangeSpot(
  files: readonly EditFile[],
  from: number,
  direction: 1 | -1,
): number | undefined {
  const spots = spotsOf(files);
  const changed = (at: number): boolean => {
    const spot = spots[at];
    const kind = spot && files[spot.file]?.lines[spot.index]?.kind;
    return kind === "add" || kind === "remove";
  };
  const starts: number[] = [];
  for (let at = 0; at < spots.length; at += 1) {
    if (!changed(at)) continue;
    const before = spots[at - 1];
    const same = before !== undefined && before.file === spots[at]!.file && changed(at - 1);
    if (!same) starts.push(at);
  }
  if (direction === 1) return starts.find((at) => at > from);
  return starts.reverse().find((at) => at < from);
}

/** A comment on a run of the edit's lines, before it goes to the agent. */
export interface EditComment {
  file: number;
  /** The first and last line commented on, in the file's lines. */
  from: number;
  to: number;
  text: string;
}

/**
 * The run of lines between two spots, cut to the file the head is in: a comment is on one file's
 * lines.
 */
export function runOf(
  spots: readonly Spot[],
  anchor: number,
  head: number,
): { file: number; from: number; to: number } | undefined {
  const at = spots[head];
  if (at === undefined) return undefined;
  const start = spots[anchor];
  const other = start !== undefined && start.file === at.file ? start.index : at.index;
  return { file: at.file, from: Math.min(other, at.index), to: Math.max(other, at.index) };
}

/** The note the answer carries: every comment with the lines it is on, as the agent reads them. */
export function noteOf(files: readonly EditFile[], comments: readonly EditComment[]): string {
  const quoted: CodeComment[] = comments.flatMap((comment, at) => {
    const file = files[comment.file];
    const place = file && quoteDiff(file.path, file.lines, comment.from, comment.to);
    return place === undefined ? [] : [{ ...place, id: `e${at}`, text: comment.text }];
  });
  return formatMessage("", quoted);
}

// Which lines the user selected to comment on, and the comment's quote of them. Lines count in the
// file's diff lines (or, in a file, its lines), so unified and split views agree.
import type { DiffLine } from "@suite/protocol/types";
import type { CodeComment } from "../../lib/comments.ts";

/** A range of lines of one view (a file's diff, or a file tab), between where it began and now. */
export interface Selection {
  view: string;
  anchor: number;
  head: number;
}

/** Pressing line `index` selects it; with `extend` (shift) it stretches the selection in that view. */
export function select(
  current: Selection | undefined,
  view: string,
  index: number,
  extend: boolean,
): Selection {
  if (extend && current?.view === view) return { ...current, head: index };
  return { view, anchor: index, head: index };
}

/** The first and last line selected. */
export function span(selection: Selection): { from: number; to: number } {
  return {
    from: Math.min(selection.anchor, selection.head),
    to: Math.max(selection.anchor, selection.head),
  };
}

export function selected(selection: Selection | undefined, view: string, index: number): boolean {
  if (selection?.view !== view) return false;
  const { from, to } = span(selection);
  return index >= from && index <= to;
}

/** What a comment needs of its place; the text is the user's, written after. */
export type Quote = Omit<CodeComment, "id" | "text">;

const MARK = { add: "+", remove: "-", context: " ", hunk: " " } as const;

function numbers(lines: readonly DiffLine[], side: "old" | "new"): number[] {
  const out: number[] = [];
  for (const line of lines) {
    const no = side === "new" ? line.new : line.old;
    if (no !== undefined) out.push(no);
  }
  return out;
}

/**
 * The lines `from` to `to` of a file's diff as a comment's place. Added and unchanged lines are
 * counted in the new text, removed lines alone in the old one. A selection of removed lines and
 * others is quoted as a diff, with the marks, and counted in the new text.
 */
export function quoteDiff(
  path: string,
  lines: readonly DiffLine[],
  from: number,
  to: number,
): Quote | undefined {
  const picked = lines.slice(from, to + 1).filter((line) => line.kind !== "hunk");
  if (picked.length === 0) return undefined;
  const removed = picked.filter((line) => line.kind === "remove").length;
  const side = removed === picked.length ? "old" : "new";
  const marked = removed > 0 && side === "new";
  const counted = numbers(picked, side);
  const code = marked
    ? picked.map((line) => `${MARK[line.kind]}${line.text}`)
    : picked.map((line) => line.text);
  return {
    path,
    side,
    start: Math.min(...counted),
    end: Math.max(...counted),
    code,
    diff: marked,
  };
}

/** The lines `from` to `to` (0-based) of a file as a comment's place. */
export function quoteFile(
  path: string,
  lines: readonly string[],
  from: number,
  to: number,
): Quote | undefined {
  const code = lines.slice(from, to + 1);
  if (code.length === 0) return undefined;
  return { path, side: "new", start: from + 1, end: from + code.length, code, diff: false };
}

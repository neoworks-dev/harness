// Reviewing a file from the keyboard, as a pull request is reviewed: a cursor on a line (or a run
// of lines) that moves and stretches, jumps between the lines the edit wrote and between the
// turn's files. Plain functions, tested without a window.
import type { Selection } from "./selection.ts";

/**
 * The cursor `delta` lines on, in a view of `total` lines; `extend` stretches it from where it
 * began instead of moving it. A cursor in another view starts at the top.
 */
export function stepCursor(
  cursor: Selection | undefined,
  view: string,
  total: number,
  delta: number,
  extend: boolean,
): Selection {
  const last = Math.max(0, total - 1);
  const clamp = (index: number): number => Math.min(last, Math.max(0, index));
  if (cursor === undefined || cursor.view !== view) return { view, anchor: 0, head: 0 };
  const head = clamp(cursor.head + delta);
  return extend ? { ...cursor, head } : { view, anchor: head, head };
}

/** The cursor on one line. */
export function cursorAt(view: string, index: number): Selection {
  return { view, anchor: index, head: index };
}

/**
 * The next line (0-based) after `from` (or before, `direction` -1) that starts a run of changed
 * lines, from the changed lines' numbers (1-based); undefined when there is none that way.
 */
export function nextChange(
  changed: ReadonlySet<number>,
  from: number,
  direction: 1 | -1,
): number | undefined {
  const starts = [...changed]
    .filter((no) => !changed.has(no - 1))
    .map((no) => no - 1)
    .sort((a, b) => a - b);
  if (direction === 1) return starts.find((index) => index > from);
  return starts.reverse().find((index) => index < from);
}

/** The file `step` on from `at` in the turn's files, or undefined past either end. */
export function stepFile(count: number, at: number, step: 1 | -1): number | undefined {
  const next = at + step;
  return next >= 0 && next < count ? next : undefined;
}

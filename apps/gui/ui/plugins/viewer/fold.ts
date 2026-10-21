// A file's diff lines with the runs of unchanged lines folded away: what the viewer draws, and
// what pressing "N unmodified lines" brings back. The lines are the whole file with its changes
// marked (`git diff -U<large>`), or hunks (a pull request), which have nothing to fold.
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";

/** Unchanged lines kept around a change. */
export const CONTEXT = 3;

/** What is drawn for one stretch of the file. `index` counts in the file's lines. */
export type Block =
  | { kind: "line"; index: number; line: DiffLine }
  | { kind: "hunk"; index: number; text: string }
  /** Unchanged lines `from` to `to` (inclusive), hidden until pressed. `id` names the fold. */
  | { kind: "fold"; id: number; count: number; from: number; to: number };

/**
 * The lines as blocks. A run of unchanged lines keeps `context` lines next to each change and folds
 * the rest, unless that would hide less than two lines or `isOpen` says the fold was opened.
 * A file with no change at all is not folded.
 */
export function fold(
  lines: readonly DiffLine[],
  isOpen: (id: number) => boolean,
  context = CONTEXT,
): Block[] {
  const blocks: Block[] = [];
  /** Lines `from` to `to` as they are. */
  const keep = (from: number, to: number): void => {
    for (let i = from; i <= to; i += 1) {
      const line = lines[i];
      if (line?.kind === "hunk") blocks.push({ kind: "hunk", index: i, text: line.text });
      else if (line !== undefined) blocks.push({ kind: "line", index: i, line });
    }
  };

  let at = 0;
  while (at < lines.length) {
    if (lines[at]?.kind !== "context") {
      keep(at, at);
      at += 1;
      continue;
    }
    // A run of unchanged lines, `at` to `last`.
    let last = at;
    while (lines[last + 1]?.kind === "context") last += 1;
    const alone = at === 0 && last === lines.length - 1;
    const head = at === 0 ? 0 : context;
    const tail = last === lines.length - 1 ? 0 : context;
    const hidden = last - at + 1 - head - tail;
    const id = at + head;
    if (alone || hidden < 2 || isOpen(id)) {
      keep(at, last);
    } else {
      keep(at, id - 1);
      blocks.push({ kind: "fold", id, count: hidden, from: id, to: last - tail });
      keep(last - tail + 1, last);
    }
    at = last + 1;
  }
  return blocks;
}

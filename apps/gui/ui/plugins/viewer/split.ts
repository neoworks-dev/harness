// The folded lines laid out for two columns: the old text on the left, the new on the right.
import type { DiffLine } from "@suite/protocol/types";
import type { Block } from "./fold.ts";

/** One side of a row: a line and where it is in the file's lines. */
export interface Cell {
  index: number;
  line: DiffLine;
}

export type SplitRow =
  | { kind: "pair"; left?: Cell; right?: Cell }
  | { kind: "hunk"; index: number; text: string }
  | { kind: "fold"; id: number; count: number; from: number; to: number };

/**
 * Unchanged lines show on both sides. A change (the removed and added lines between two stretches of
 * unchanged ones) pairs its removed lines with its added lines from the top, so a rewritten line sits
 * opposite its old version; the side with fewer is left empty for the rest.
 */
export function splitRows(blocks: readonly Block[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let removed: Cell[] = [];
  let added: Cell[] = [];

  const flush = (): void => {
    const count = Math.max(removed.length, added.length);
    for (let i = 0; i < count; i += 1)
      rows.push({ kind: "pair", left: removed[i], right: added[i] });
    removed = [];
    added = [];
  };

  for (const block of blocks) {
    if (block.kind !== "line") {
      flush();
      rows.push(block);
      continue;
    }
    const cell = { index: block.index, line: block.line };
    if (block.line.kind === "remove") removed.push(cell);
    else if (block.line.kind === "add") added.push(cell);
    else {
      flush();
      rows.push({ kind: "pair", left: cell, right: cell });
    }
  }
  flush();
  return rows;
}

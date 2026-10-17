// A diff component's lines as the chat draws them: counted, cut to a preview, and measured for
// the width of their line numbers.
import type { DiffLine } from "@suite/protocol/types";

export interface DiffStats {
  added: number;
  removed: number;
}

export function diffStats(lines: readonly DiffLine[]): DiffStats {
  let added = 0;
  let removed = 0;
  for (const line of lines) {
    if (line.kind === "add") added += 1;
    else if (line.kind === "remove") removed += 1;
  }
  return { added, removed };
}

/**
 * The first `limit` items and how many were left out. A hint saying "N more" takes a line
 * itself, so cutting a single line would save nothing: then everything is kept.
 */
export function clip<T>(
  items: readonly T[],
  limit: number,
): { shown: readonly T[]; hidden: number } {
  if (items.length <= limit + 1) return { shown: items, hidden: 0 };
  return { shown: items.slice(0, limit), hidden: items.length - limit };
}

/** Width in characters of the line-number column for these lines. */
export function gutterWidth(lines: readonly DiffLine[]): number {
  let widest = 1;
  for (const line of lines) {
    for (const no of [line.old, line.new]) {
      if (no !== undefined) widest = Math.max(widest, String(no).length);
    }
  }
  return widest;
}

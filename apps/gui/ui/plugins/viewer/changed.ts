// Which lines of a file an edit wrote, so the whole file can be shown with them marked. Plain,
// tested without a window.
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";

/** The added lines' numbers in the new text (1-based), in order. */
export function addedLines(lines: readonly DiffLine[]): number[] {
  const out: number[] = [];
  for (const line of lines) if (line.kind === "add" && line.new !== undefined) out.push(line.new);
  return out;
}

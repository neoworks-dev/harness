// Sizes the viewer's lines share: a line of code is 13px mono, 20px high, and its numbers take the
// room of the longest one.
import type { DiffLine } from "@suite/protocol/types";
import type { ViewerColors } from "../../lib/diff-colors.ts";

/** The width of one monospace character at 13px. */
export const CHAR_WIDTH = 7.8;

/** A column of line numbers `digits` wide. */
export function numberWidth(digits: number): number {
  return Math.round(digits * CHAR_WIDTH) + 16;
}

const NBSP = "\u00a0";

/**
 * A line of code as text that may wrap at its spaces yet keeps its indentation and the runs of
 * spaces inside it. `pre` text does not wrap, and plain text collapses its spaces.
 */
export function wrappable(text: string): string {
  return text
    .replace(/\t/g, NBSP.repeat(4))
    .replace(/^ +/, (lead) => NBSP.repeat(lead.length))
    .replace(/ {2,}/g, (run) => `${NBSP.repeat(run.length - 1)} `);
}

/** The mark in front of a changed line's text. */
export function markOf(kind: DiffLine["kind"]): string {
  if (kind === "add") return "+";
  if (kind === "remove") return "−";
  return "";
}

/** The background of a line's text and of its numbers: its change, or the selection over it. */
export function backgrounds(
  colors: ViewerColors,
  kind: DiffLine["kind"],
  selected: boolean,
): { body: string; numbers: string } {
  if (selected) return { body: colors.selected, numbers: colors.selectedNumbers };
  if (kind === "add") return { body: colors.add, numbers: colors.addNumbers };
  if (kind === "remove") return { body: colors.remove, numbers: colors.removeNumbers };
  return { body: "transparent", numbers: colors.numbers };
}

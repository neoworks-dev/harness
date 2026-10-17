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
 * spaces inside it. `pre` text does not wrap, and plain text collapses its spaces. A run in the
 * middle of a line (`start` false) may wrap at the spaces it starts with.
 */
export function wrappable(text: string, start = true): string {
  const tabbed = text.replace(/\t/g, NBSP.repeat(4));
  const indented = start ? tabbed.replace(/^ +/, (lead) => NBSP.repeat(lead.length)) : tabbed;
  return indented.replace(/ {2,}/g, (run) => `${NBSP.repeat(run.length - 1)} `);
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
): { body: string; numbers: string; mark: string } {
  if (selected)
    return { body: colors.selected, numbers: colors.selectedNumbers, mark: colors.selected };
  if (kind === "add") return { body: colors.add, numbers: colors.addNumbers, mark: colors.addMark };
  if (kind === "remove") {
    return { body: colors.remove, numbers: colors.removeNumbers, mark: colors.removeMark };
  }
  return { body: "transparent", numbers: colors.numbers, mark: "transparent" };
}

/** How many squares a file's diffstat bar has. */
export const BAR_SQUARES = 5;

/** A diffstat as squares, as GitHub draws it: the added lines' share green, the removed lines' red. */
export function statBar(added: number, removed: number): ("add" | "remove" | "none")[] {
  const total = added + removed;
  if (total === 0) return Array.from({ length: BAR_SQUARES }, () => "none");
  let green = Math.round((added / total) * BAR_SQUARES);
  if (added > 0 && green === 0) green = 1;
  if (removed > 0 && green === BAR_SQUARES) green = BAR_SQUARES - 1;
  return Array.from({ length: BAR_SQUARES }, (_, i) => (i < green ? "add" : "remove"));
}

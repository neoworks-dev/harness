// What the item renderers share.
import { blend, type Palette } from "../../lib/palette.ts";

/** The context key under which a frame says that what is drawn in it is framed already. */
export const FRAMED = Symbol("framed");

/** Width of the icon column (spinner, ✓) that a tool call's result hangs from. */
export const GUTTER = 24;

/** Where a work row's detail starts: under its label, past the icon column and the gap. */
export const DETAIL_INDENT = GUTTER + 6;

/** Lines of a diff or code a question shows before its buttons (Ctrl-O shows all). */
export const PREVIEW_LINES = 12;

/** The rules under the "Working for" and "Worked for" rows: the border at 60%. */
export function softBorder(p: Palette): string {
  return blend(p.bg, p.border, 60);
}

/** The agent's prose and the text of errors: 80% of the text colour. */
export function proseColor(p: Palette): string {
  return blend(p.bg, p.fg, 80);
}

/** Code and paths in the work rows: 85% of the text colour. */
export function codeColor(p: Palette): string {
  return blend(p.bg, p.fg, 85);
}

/** Line height of the chat's prose: 1.625 times the size. */
export function proseLineHeight(size: number): number {
  return Math.round(size * 1.625 * 100) / 100;
}

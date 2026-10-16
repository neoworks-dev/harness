// What the card's rows share. The values follow t3code's thread details panel.
import type { Style } from "@neoworks/svelte-gpui/ui";
import { blend, type Palette } from "../../lib/palette.ts";

export const WIDTH = 280;
export const ROW = 32;

/** The rows' text: the foreground at 80% over the card. */
export function textOf(p: Palette): string {
  return blend(p.surface, p.fg, 80);
}

/** A row or a half of a split button: 32px, with the icon and the label 10px apart. */
export function rowStyle(p: Palette): Style {
  return {
    direction: "row",
    align: "center",
    gap: 10,
    height: ROW,
    paddingX: 10,
    radius: 8,
    color: textOf(p),
    fontSize: 13,
    fontWeight: "medium",
  };
}

/** A row under the pointer. `p.hover` is nearly the card's own colour, so the raised one. */
export function rowHover(p: Palette): Style {
  return { background: p.raised };
}

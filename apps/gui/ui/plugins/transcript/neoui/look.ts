// Colours of a neoui doc, from the window's palette.
import type { Tone } from "@neoworks-dev/neo-protocol/types";
import { blend, type Palette } from "../../../lib/palette.ts";

/** The colour a tone stands for; the text colour for none. */
export function toneColor(tone: Tone | undefined, p: Palette): string {
  return tone === undefined ? p.fg : p[tone];
}

/** `color` washed into the surface: the fill behind a tag, a callout or a selected option. */
export function tint(p: Palette, color: string, percent = 16): string {
  return blend(p.surface, color, percent);
}

/** A card's outline: the border colour, or the tone's colour toned down. */
export function outline(p: Palette, tone: Tone | undefined): string {
  return tone === undefined ? p.border : blend(p.border, p[tone], 65);
}

/** The frame of a field: a thin border that turns accent while it has the keyboard and danger on an error. */
export function fieldLook(
  p: Palette,
  state: { focused?: boolean; error?: boolean },
): {
  background: string;
  borderWidth: number;
  borderColor: string;
  radius: number;
} {
  let color = p.border;
  if (state.error === true) color = p.danger;
  else if (state.focused === true) color = p.accent;
  return { background: p.bg, borderWidth: 1, borderColor: color, radius: p.radius.s };
}

/** The colours of a button by its variant. */
export function buttonLook(
  p: Palette,
  variant: string | undefined,
): { background: string; hover: string; border: string; color: string } {
  if (variant === "primary") {
    return {
      background: p.accent,
      hover: blend(p.accent, p.fg, 18),
      border: p.accent,
      color: p.bg,
    };
  }
  if (variant === "danger") {
    return {
      background: tint(p, p.danger, 14),
      hover: tint(p, p.danger, 26),
      border: blend(p.border, p.danger, 70),
      color: p.danger,
    };
  }
  return { background: p.surface, hover: p.raised, border: p.border, color: p.fg };
}

// The colours and text sizes the window draws with, from the daemon's theme (protocol/themes).
// Until the daemon sends one, the default theme's values are used.
import type { Theme } from "@suite/protocol/types";

export interface Palette {
  fg: string;
  bg: string;
  /** Cards, code, the side panels. */
  surface: string;
  border: string;
  accent: string;
  muted: string;
  success: string;
  danger: string;
  /** The band behind the user's own messages and the input: bg with a little fg mixed in. */
  band: string;
  /** The sessions column: its background, rows under the pointer and the shown row, its border, icons. */
  side: { bg: string; hover: string; active: string; border: string; icon: string; muted: string };
  /** The sessions column's background. */
  sidebar: string;
  /** A row under the pointer. */
  hover: string;
  /** The shown row, and the user's message bubble. */
  raised: string;
  /** Corner radii in pixels: rows and buttons, then cards and the composer. */
  radius: { s: number; m: number; l: number };
  /** Pixels. */
  space: { s: number; m: number; l: number };
  text: { title: number; body: number; code: number };
}

/** protocol/themes/default.json; a test keeps the two in step. */
export const DEFAULT_THEME: Theme = {
  name: "default-dark",
  colors: {
    fg: "#e6e6e6",
    bg: "#16181d",
    surface: "#1f2229",
    border: "#343844",
    accent: "#7aa2f7",
    muted: "#8a8f9c",
    success: "#9ece6a",
    danger: "#f7768e",
  },
  spacing: { s: 4, m: 8, l: 16 },
  text: {
    title: { size: 15, weight: 600, mono: false },
    body: { size: 14, weight: 400, mono: false },
    code: { size: 13, weight: 400, mono: true },
  },
};

function channels(hex: string): [number, number, number] | undefined {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!match) return undefined;
  return [parseInt(match[1] ?? "", 16), parseInt(match[2] ?? "", 16), parseInt(match[3] ?? "", 16)];
}

/** `over` mixed into `base` by `percent`; `base` if either isn't #rrggbb. */
export function blend(base: string, over: string, percent: number): string {
  const a = channels(base);
  const b = channels(over);
  if (!a || !b) return base;
  const mix = (x: number, y: number): number =>
    Math.round((x * (100 - percent) + y * percent) / 100);
  const hex = (n: number): string => n.toString(16).padStart(2, "0");
  return `#${hex(mix(a[0], b[0]))}${hex(mix(a[1], b[1]))}${hex(mix(a[2], b[2]))}`;
}

/**
 * The window's neutral greys, near-black and without the theme's tint. The theme still decides
 * the accent and the success and danger colours.
 */
const NEUTRAL = {
  fg: "#ededed",
  bg: "#0e0e0f",
  surface: "#161617",
  border: "#262628",
  muted: "#9a9a9f",
  sidebar: "#0b0b0c",
  hover: "#1a1a1c",
  raised: "#222225",
};

/** The sessions column, a shade darker than the window. */
const SIDE = {
  bg: NEUTRAL.sidebar,
  hover: NEUTRAL.hover,
  active: NEUTRAL.raised,
  border: NEUTRAL.border,
  icon: "#6a6a70",
  muted: NEUTRAL.muted,
};

export function paletteOf(theme: Theme = DEFAULT_THEME): Palette {
  const { colors, spacing, text } = theme;
  return {
    ...colors,
    ...NEUTRAL,
    band: NEUTRAL.raised,
    side: SIDE,
    radius: { s: 8, m: 10, l: 18 },
    space: { s: spacing.s, m: spacing.m, l: spacing.l },
    text: { title: text.title.size, body: text.body.size, code: text.code.size },
  };
}

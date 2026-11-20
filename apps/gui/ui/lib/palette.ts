// The colours and text sizes the window draws with, from the daemon's theme (protocol/themes).
// Until the daemon sends one, the default theme's values are used.
import type { Theme } from "@neoworks-dev/neo-protocol/types";

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
  /** Work in progress, as on a running session. Used sparingly. */
  info: string;
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
  /** The user's message bubble. */
  message: string;
  /** The agent's prose: the text colour, a step back. */
  prose: string;
  /** Corner radii in pixels: rows and buttons, then cards and the composer. */
  radius: { s: number; m: number; l: number };
  /** Pixels. */
  space: { s: number; m: number; l: number };
  text: { title: number; body: number; code: number };
  /** What code is highlighted with: a colour for each kind of word, light enough for the dark greys. */
  syntax: Syntax;
}

export interface Syntax {
  keyword: string;
  string: string;
  number: string;
  comment: string;
  function: string;
  type: string;
  constant: string;
  tag: string;
  attr: string;
  property: string;
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
  bg: "#111113",
  surface: "#19191b",
  border: "#262628",
  muted: "#9a9a9f",
  sidebar: "#0b0b0c",
  hover: "#1a1a1c",
  raised: "#222225",
};

/**
 * The accent, and what is running: the foreground's white, not a hue. The window marks the
 * selected, the focused and the primary in white; colour is left to success, danger and code.
 */
const ACCENT = NEUTRAL.fg;
const INFO = NEUTRAL.fg;

/** The sessions column, a shade darker than the window. */
const SIDE = {
  bg: NEUTRAL.sidebar,
  hover: NEUTRAL.hover,
  active: NEUTRAL.raised,
  border: NEUTRAL.border,
  icon: "#6a6a70",
  muted: NEUTRAL.muted,
};

/**
 * The theme's own colours where it has the hue (the accent for functions, success for strings,
 * danger for tags), and a few more that no theme names, tuned for the near-black greys.
 */
function syntaxOf(colors: Theme["colors"]): Syntax {
  return {
    keyword: "#bb9af7",
    string: colors.success,
    number: "#ff9e64",
    comment: blend(NEUTRAL.muted, NEUTRAL.bg, 22),
    function: colors.accent,
    type: "#2ac3de",
    constant: "#e0af68",
    tag: colors.danger,
    attr: "#73daca",
    property: "#7dcfff",
  };
}

export function paletteOf(theme: Theme = DEFAULT_THEME): Palette {
  const { colors, spacing, text } = theme;
  return {
    ...colors,
    ...NEUTRAL,
    accent: ACCENT,
    info: INFO,
    band: NEUTRAL.raised,
    side: SIDE,
    message: "#1a1a1c",
    prose: blend(NEUTRAL.fg, NEUTRAL.bg, 15),
    radius: { s: 8, m: 10, l: 18 },
    space: { s: spacing.s, m: spacing.m, l: spacing.l },
    text: { title: text.title.size, body: text.body.size, code: text.code.size },
    syntax: syntaxOf(colors),
  };
}

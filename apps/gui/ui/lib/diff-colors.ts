// The backgrounds and text colours a diff is drawn with: the added and removed colours mixed
// into the surface, so changed lines are marked without shouting.
import { blend, type Palette } from "./palette.ts";

export interface DiffColors {
  add: string;
  remove: string;
  /** The line-number gutter and the marks next to them. */
  gutter: string;
}

export function diffColors(p: Palette): DiffColors {
  return {
    add: blend(p.surface, p.success, 20),
    remove: blend(p.surface, p.danger, 20),
    gutter: p.muted,
  };
}

export interface ViewerColors {
  /** A changed line's background, and its line numbers' (a little stronger). */
  add: string;
  addNumbers: string;
  remove: string;
  removeNumbers: string;
  /** The line numbers of unchanged lines. */
  numbers: string;
  /** The empty half of a split row. */
  empty: string;
  /** The lines selected for a comment, and their line numbers. */
  selected: string;
  selectedNumbers: string;
}

/** What the viewer's diff and file lines are drawn with: the changes mixed into the card's own colour. */
export function viewerColors(p: Palette): ViewerColors {
  return {
    add: blend(p.bg, p.success, 15),
    addNumbers: blend(p.bg, p.success, 25),
    remove: blend(p.bg, p.danger, 15),
    removeNumbers: blend(p.bg, p.danger, 25),
    numbers: blend(p.bg, p.fg, 4),
    empty: blend(p.bg, p.fg, 2),
    selected: blend(p.bg, p.accent, 20),
    selectedNumbers: blend(p.bg, p.accent, 32),
  };
}

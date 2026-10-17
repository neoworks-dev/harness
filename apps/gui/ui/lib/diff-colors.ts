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
  /** The words of a rewritten line that changed. */
  addMark: string;
  remove: string;
  removeNumbers: string;
  removeMark: string;
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
  const known = viewer.get(p);
  if (known !== undefined) return known;
  const made = {
    add: blend(p.bg, p.success, 15),
    addNumbers: blend(p.bg, p.success, 25),
    addMark: blend(p.bg, p.success, 38),
    remove: blend(p.bg, p.danger, 15),
    removeNumbers: blend(p.bg, p.danger, 25),
    removeMark: blend(p.bg, p.danger, 38),
    numbers: blend(p.bg, p.fg, 4),
    empty: blend(p.bg, p.fg, 2),
    selected: blend(p.bg, p.accent, 20),
    selectedNumbers: blend(p.bg, p.accent, 32),
  };
  viewer.set(p, made);
  return made;
}

/** One set per palette: a row asks for it as it is drawn. */
const viewer = new WeakMap<Palette, ViewerColors>();

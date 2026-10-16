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

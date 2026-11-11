// The width of the conversation's column, for what is drawn in it and has to know how much room it has.
import { getContext } from "svelte";

export const COLUMN = Symbol("column");

export interface ColumnSize {
  /** Pixels, as the column's children count them (before its zoom). */
  readonly width: number;
  /**
   * How much larger the children are drawn. What they measure (in the window's pixels) is this
   * much larger than the widths they set: divide by it.
   */
  readonly zoom: number;
}

/** The column the component is drawn in (call it while initialising); a default width outside one. */
export function useColumn(): ColumnSize {
  return getContext<ColumnSize | undefined>(COLUMN) ?? { width: 640, zoom: 1 };
}

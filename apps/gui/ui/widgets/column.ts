// The width of the conversation's column, for what is drawn in it and has to know how much room it has.
import { getContext } from "svelte";

export const COLUMN = Symbol("column");

export interface ColumnSize {
  /** Pixels. */
  readonly width: number;
}

/** The column the component is drawn in (call it while initialising); a default width outside one. */
export function useColumn(): ColumnSize {
  return getContext<ColumnSize | undefined>(COLUMN) ?? { width: 640 };
}

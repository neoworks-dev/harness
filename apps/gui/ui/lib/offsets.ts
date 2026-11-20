// Where the rows of a list sit, from their heights as drawn and, for those not drawn yet, the
// estimate the list counts them as: so a scroll offset names the row there. The list itself only
// scrolls to an index; the chat and the terminal panel need the way back to keep each other in
// step. Plain, tested without a window.

export class Offsets {
  private readonly heights = new Map<string, number>();

  /** A row was drawn this tall. */
  set(key: string, height: number): void {
    this.heights.set(key, height);
  }

  /** The index of the row at `offset` (pixels from the top of the list), or -1 for none. */
  indexAt(offset: number, keys: readonly string[], estimate: (index: number) => number): number {
    let top = 0;
    for (let index = 0; index < keys.length; index += 1) {
      const height = this.heights.get(keys[index]!) ?? estimate(index);
      if (offset < top + height) return index;
      top += height;
    }
    return keys.length - 1;
  }
}

/**
 * The newest of `wanted` at or before `index`, walking back through `ids` (each row's ids): the
 * command a window ending at that row last showed. Undefined when there is none.
 */
export function lastAtOrBefore(
  index: number,
  ids: readonly (readonly string[])[],
  wanted: (id: string) => boolean,
): string | undefined {
  for (let at = Math.min(index, ids.length - 1); at >= 0; at -= 1) {
    const row = ids[at] ?? [];
    for (let i = row.length - 1; i >= 0; i -= 1) if (wanted(row[i]!)) return row[i];
  }
  return undefined;
}

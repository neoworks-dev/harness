// Where the rows of a long list sit, and which of them are near the window. A list whose rows
// have heights it can tell beforehand (a line of code is 20px, a header 36) needs no measuring:
// the tops are a running sum, and the rows to build are a search in it.

/** Where each of `count` rows begins, and (last) where the list ends. */
export function tops(count: number, heightAt: (index: number) => number): number[] {
  const out = new Array<number>(count + 1);
  let y = 0;
  for (let i = 0; i < count; i += 1) {
    out[i] = y;
    y += heightAt(i);
  }
  out[count] = y;
  return out;
}

/** The row at offset `y`: the last one beginning at or above it. */
export function indexAt(starts: readonly number[], y: number): number {
  const count = starts.length - 1;
  if (count <= 0) return 0;
  let low = 0;
  let high = count - 1;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if ((starts[mid] ?? 0) <= y) low = mid;
    else high = mid - 1;
  }
  return low;
}

/**
 * The rows to build for a window `viewport` tall at offset `y`: `first` up to (not including)
 * `last`, with `overscan` rows past each edge.
 */
export function visible(
  starts: readonly number[],
  y: number,
  viewport: number,
  overscan: number,
): { first: number; last: number } {
  const count = starts.length - 1;
  if (count <= 0) return { first: 0, last: 0 };
  const first = Math.max(0, indexAt(starts, y) - overscan);
  const last = Math.min(count, indexAt(starts, y + viewport) + 1 + overscan);
  return { first, last };
}

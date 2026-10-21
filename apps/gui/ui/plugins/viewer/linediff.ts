// The difference between two texts as the whole file with its changes marked, for the edits an
// agent made (the tool call carries the old and the new text, not a diff).
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";

/** The most cells of the table that finds the longest common lines; past it the middle is replaced whole. */
const MAX_CELLS = 4_000_000;

/** A text's lines; the newline that ends the last one does not start another. */
export function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** The lines of `a` and of `b` that stay, as pairs of positions, from the longest common run. */
function common(a: readonly string[], b: readonly string[]): [number, number][] {
  const n = a.length;
  const m = b.length;
  if (n * m > MAX_CELLS) return [];
  // table[i][j]: the longest common length of a[i..] and b[j..].
  const width = m + 1;
  const table = new Uint32Array((n + 1) * width);
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      table[i * width + j] =
        a[i] === b[j]
          ? (table[(i + 1) * width + j + 1] ?? 0) + 1
          : Math.max(table[(i + 1) * width + j] ?? 0, table[i * width + j + 1] ?? 0);
    }
  }
  const pairs: [number, number][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      pairs.push([i, j]);
      i += 1;
      j += 1;
    } else if ((table[(i + 1) * width + j] ?? 0) >= (table[i * width + j + 1] ?? 0)) {
      i += 1;
    } else {
      j += 1;
    }
  }
  return pairs;
}

/**
 * Every line of both texts: unchanged ones as context, the rest as removed (old text) and added
 * (new text) lines, numbered in their own text. Text only in `newText` (a new file) is all added.
 */
export function lineDiff(oldText: string | undefined, newText: string): DiffLine[] {
  const a = splitLines(oldText ?? "");
  const b = splitLines(newText);
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head += 1;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail += 1;
  }
  const midA = a.slice(head, a.length - tail);
  const midB = b.slice(head, b.length - tail);

  const out: DiffLine[] = [];
  let oldNo = 1;
  let newNo = 1;
  const context = (text: string): void => {
    out.push({ kind: "context", text, old: oldNo, new: newNo });
    oldNo += 1;
    newNo += 1;
  };
  const remove = (text: string): void => {
    out.push({ kind: "remove", text, old: oldNo });
    oldNo += 1;
  };
  const add = (text: string): void => {
    out.push({ kind: "add", text, new: newNo });
    newNo += 1;
  };

  for (const text of a.slice(0, head)) context(text);
  let i = 0;
  let j = 0;
  for (const [pi, pj] of common(midA, midB)) {
    while (i < pi) remove(midA[i++] ?? "");
    while (j < pj) add(midB[j++] ?? "");
    context(midA[i] ?? "");
    i += 1;
    j += 1;
  }
  while (i < midA.length) remove(midA[i++] ?? "");
  while (j < midB.length) add(midB[j++] ?? "");
  for (const text of a.slice(a.length - tail)) context(text);
  return out;
}

import { expect, test } from "bun:test";
import type { DiffLine } from "@suite/protocol/types";
import { fold } from "../ui/plugins/viewer/fold.ts";
import { buildRows, fileAt, offsets, withEditor, type Layout } from "../ui/plugins/viewer/rows.ts";
import { quoteDiff, quoteFile, select, selected, span } from "../ui/plugins/viewer/selection.ts";
import { splitRows } from "../ui/plugins/viewer/split.ts";

/** A file of `n` unchanged lines with the lines in `changed` (1-based) replaced. */
function file(n: number, changed: number[]): DiffLine[] {
  const lines: DiffLine[] = [];
  let oldNo = 1;
  let newNo = 1;
  for (let i = 1; i <= n; i += 1) {
    if (changed.includes(i)) {
      lines.push({ kind: "remove", text: `old ${i}`, old: oldNo++ });
      lines.push({ kind: "add", text: `new ${i}`, new: newNo++ });
    } else {
      lines.push({ kind: "context", text: `line ${i}`, old: oldNo++, new: newNo++ });
    }
  }
  return lines;
}

const closed = (): boolean => false;

/** Compact: each block as one word. */
function shape(lines: DiffLine[], isOpen: (id: number) => boolean = closed): string[] {
  return fold(lines, isOpen).map((block) => {
    if (block.kind === "fold") return `fold${block.count}`;
    if (block.kind === "hunk") return "hunk";
    return block.line.text;
  });
}

test("long runs of unchanged lines fold, keeping three around each change", () => {
  expect(shape(file(20, [10]))).toEqual([
    "fold6",
    "line 7",
    "line 8",
    "line 9",
    "old 10",
    "new 10",
    "line 11",
    "line 12",
    "line 13",
    "fold7",
  ]);
  expect(shape(file(30, [2, 28]))).toEqual([
    "line 1",
    "old 2",
    "new 2",
    "line 3",
    "line 4",
    "line 5",
    "fold19",
    "line 25",
    "line 26",
    "line 27",
    "old 28",
    "new 28",
    "line 29",
    "line 30",
  ]);
});

test("a fold that would hide one line, or a file with no change, is left open", () => {
  const folded = (lines: DiffLine[]): boolean =>
    shape(lines).some((word) => word.startsWith("fold"));
  expect(folded(file(30, []))).toBe(false);
  // Four unchanged lines before the change hide one; five hide two.
  expect(folded(file(8, [5]))).toBe(false);
  expect(folded(file(9, [6]))).toBe(true);
});

test("an opened fold shows its lines", () => {
  const lines = file(20, [10]);
  const [first] = fold(lines, closed).filter((block) => block.kind === "fold");
  expect(first?.kind === "fold" && first.from).toBe(0);
  const opened = fold(lines, (id) => id === 0);
  expect(opened.filter((block) => block.kind === "fold")).toHaveLength(1);
  expect(opened.filter((block) => block.kind === "line")).toHaveLength(
    fold(lines, closed).filter((block) => block.kind === "line").length + 6,
  );
});

test("hunk lines pass through and break runs", () => {
  const lines: DiffLine[] = [
    { kind: "hunk", text: "@@ -1,2 +1,2 @@" },
    { kind: "context", text: "a", old: 1, new: 1 },
    { kind: "remove", text: "b", old: 2 },
    { kind: "add", text: "c", new: 2 },
  ];
  expect(shape(lines)).toEqual(["hunk", "a", "b", "c"]);
});

test("split rows pair removed and added lines from the top, context on both sides", () => {
  const lines: DiffLine[] = [
    { kind: "context", text: "a", old: 1, new: 1 },
    { kind: "remove", text: "b", old: 2 },
    { kind: "remove", text: "c", old: 3 },
    { kind: "add", text: "B", new: 2 },
    { kind: "context", text: "d", old: 4, new: 3 },
    { kind: "add", text: "e", new: 4 },
  ];
  const rows = splitRows(fold(lines, closed));
  const text = rows.map((row) =>
    row.kind === "pair" ? `${row.left?.line.text ?? "_"}|${row.right?.line.text ?? "_"}` : row.kind,
  );
  expect(text).toEqual(["a|a", "b|B", "c|_", "d|d", "_|e"]);
  const pair = rows[1];
  expect(pair?.kind === "pair" && [pair.left?.index, pair.right?.index]).toEqual([1, 3]);
});

const layout = (over: Partial<Layout> = {}): Layout => ({
  split: false,
  collapsed: new Set(),
  isOpen: () => false,
  ...over,
});

test("rows are a header and the folded lines of each file; a collapsed file is its header", () => {
  const files = [
    { path: "a.ts", lines: file(20, [10]) },
    { path: "b.ts", lines: [] },
  ];
  const rows = buildRows(files, layout());
  expect(rows[0]).toMatchObject({ kind: "file", path: "a.ts", added: 1, removed: 1 });
  expect(rows.filter((row) => row.kind === "fold")).toHaveLength(2);
  expect(rows.at(-2)).toMatchObject({ kind: "file", path: "b.ts" });
  expect(rows.at(-1)).toMatchObject({ kind: "note" });
  const collapsed = buildRows(files, layout({ collapsed: new Set(["a.ts"]) }));
  expect(collapsed.map((row) => row.kind)).toEqual(["file", "file", "note"]);
  expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
});

test("the split layout has a row per pair", () => {
  const rows = buildRows([{ path: "a.ts", lines: file(8, [4]) }], layout({ split: true }));
  expect(rows.map((row) => row.kind)).toEqual(["file", ...Array<string>(8).fill("pair")]);
});

test("the editor goes under the last selected line, or at the end of the file's rows", () => {
  const files = [
    { path: "a.ts", lines: file(8, [4]) },
    { path: "b.ts", lines: file(4, [2]) },
  ];
  const rows = buildRows(files, layout());
  // Line index 3 is the removed line 4 (lines 0-2 are context).
  const withIt = withEditor(rows, 0, 3);
  const at = withIt.findIndex((row) => row.kind === "editor");
  expect(withIt[at - 1]).toMatchObject({ kind: "line", index: 3 });
  expect(withIt[at]?.file).toBe(0);
  const far = withEditor(rows, 1, 999);
  expect(far.at(-1)?.kind).toBe("editor");
});

test("the sticky header follows the file at the top of the list", () => {
  const files = [
    { path: "a.ts", lines: file(8, [4]) },
    { path: "b.ts", lines: file(8, [4]) },
  ];
  const rows = buildRows(files, layout());
  const starts = offsets(rows, 80);
  expect(starts).toHaveLength(rows.length + 1);
  expect(fileAt(rows, starts, 0)).toBe(0);
  const second = rows.findIndex((row) => row.kind === "file" && row.file === 1);
  expect(fileAt(rows, starts, (starts[second] ?? 0) - 1)).toBe(0);
  expect(fileAt(rows, starts, starts[second] ?? 0)).toBe(1);
  expect(fileAt(rows, starts, 99999)).toBe(1);
  expect(fileAt([], [0], 0)).toBeUndefined();
});

test("a wrapped line is guessed taller", () => {
  const rows = buildRows(
    [{ path: "a.ts", lines: [{ kind: "add", text: "x".repeat(100), new: 1 }] }],
    layout(),
  );
  expect(offsets(rows, 40).at(-1)).toBe(32 + 3 * 20);
  expect(offsets(rows, 400).at(-1)).toBe(32 + 20);
});

test("pressing a line selects it, shift stretches the range within the same view", () => {
  const first = select(undefined, "a.ts", 4, false);
  expect(span(first)).toEqual({ from: 4, to: 4 });
  const stretched = select(first, "a.ts", 1, true);
  expect(span(stretched)).toEqual({ from: 1, to: 4 });
  expect(stretched.anchor).toBe(4);
  expect(select(stretched, "b.ts", 9, true)).toEqual({ view: "b.ts", anchor: 9, head: 9 });
  expect(select(stretched, "a.ts", 7, false)).toEqual({ view: "a.ts", anchor: 7, head: 7 });
  expect(selected(stretched, "a.ts", 2)).toBe(true);
  expect(selected(stretched, "a.ts", 5)).toBe(false);
  expect(selected(stretched, "b.ts", 2)).toBe(false);
  expect(selected(undefined, "a.ts", 2)).toBe(false);
});

test("a selection in a diff is quoted by the text it counts in", () => {
  const lines = file(6, [3]);
  // 0 line 1, 1 line 2, 2 old 3, 3 new 3, 4 line 4
  expect(quoteDiff("a.ts", lines, 0, 1)).toEqual({
    path: "a.ts",
    side: "new",
    start: 1,
    end: 2,
    code: ["line 1", "line 2"],
    diff: false,
  });
  expect(quoteDiff("a.ts", lines, 2, 2)).toEqual({
    path: "a.ts",
    side: "old",
    start: 3,
    end: 3,
    code: ["old 3"],
    diff: false,
  });
  expect(quoteDiff("a.ts", lines, 3, 3)?.code).toEqual(["new 3"]);
  const mixed = quoteDiff("a.ts", lines, 1, 4);
  expect(mixed).toEqual({
    path: "a.ts",
    side: "new",
    start: 2,
    end: 4,
    code: [" line 2", "-old 3", "+new 3", " line 4"],
    diff: true,
  });
  expect(quoteDiff("a.ts", [{ kind: "hunk", text: "@@" }], 0, 0)).toBeUndefined();
});

test("a selection in a file is quoted by its lines", () => {
  expect(quoteFile("a.ts", ["a", "b", "c", "d"], 1, 2)).toEqual({
    path: "a.ts",
    side: "new",
    start: 2,
    end: 3,
    code: ["b", "c"],
    diff: false,
  });
  expect(quoteFile("a.ts", ["a"], 4, 5)).toBeUndefined();
});

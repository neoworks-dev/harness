// The diff tab's list as rows: for each file a header, then its lines (one per row, or a pair in the
// split layout) with the unchanged runs folded, and the comment's editor under the selection. Plain
// data, so tests reach it; the components draw a row by its kind.
import type { DiffLine } from "@suite/protocol/types";
import { diffStats } from "../../lib/diff.ts";
import type { FileDiff } from "../../lib/unified.ts";
import { fold } from "./fold.ts";
import { splitRows, type Cell } from "./split.ts";

export const HEADER_HEIGHT = 32;
export const LINE_HEIGHT = 20;
export const FOLD_HEIGHT = 24;
export const EDITOR_HEIGHT = 132;

interface Base {
  /** Names the row for the list, which keeps a row's height with it. */
  key: string;
  /** Which of the files the row belongs to. */
  file: number;
}

export type Row =
  | (Base & { kind: "file"; path: string; added: number; removed: number; collapsed: boolean })
  | (Base & { kind: "line"; index: number; line: DiffLine })
  | (Base & { kind: "pair"; left?: Cell; right?: Cell })
  | (Base & { kind: "fold"; id: number; count: number })
  | (Base & { kind: "hunk"; text: string })
  | (Base & { kind: "note"; text: string })
  | (Base & { kind: "editor" });

export interface Layout {
  split: boolean;
  /** Paths whose lines are hidden. */
  collapsed: ReadonlySet<string>;
  /** Whether the fold `id` of `path` was opened. */
  isOpen: (path: string, id: number) => boolean;
}

/** The rows of every file, one after the other. */
export function buildRows(files: readonly FileDiff[], layout: Layout): Row[] {
  const rows: Row[] = [];
  files.forEach((file, f) => {
    const stats = diffStats(file.lines);
    const collapsed = layout.collapsed.has(file.path);
    rows.push({
      kind: "file",
      key: `${f}:file`,
      file: f,
      path: file.path,
      added: stats.added,
      removed: stats.removed,
      collapsed,
    });
    if (collapsed) return;
    if (file.lines.length === 0) {
      rows.push({ kind: "note", key: `${f}:note`, file: f, text: "No changes to show" });
      return;
    }
    const blocks = fold(file.lines, (id) => layout.isOpen(file.path, id));
    if (!layout.split) {
      for (const block of blocks) {
        if (block.kind === "line") {
          const { index, line } = block;
          rows.push({ kind: "line", key: `${f}:${index}`, file: f, index, line });
        } else if (block.kind === "hunk") {
          rows.push({ kind: "hunk", key: `${f}:h${block.index}`, file: f, text: block.text });
        } else {
          rows.push({
            kind: "fold",
            key: `${f}:f${block.id}`,
            file: f,
            id: block.id,
            count: block.count,
          });
        }
      }
      return;
    }
    for (const row of splitRows(blocks)) {
      if (row.kind === "pair") {
        const key = `${f}:${row.left?.index ?? "-"}/${row.right?.index ?? "-"}`;
        rows.push({ kind: "pair", key, file: f, left: row.left, right: row.right });
      } else if (row.kind === "hunk") {
        rows.push({ kind: "hunk", key: `${f}:h${row.index}`, file: f, text: row.text });
      } else {
        rows.push({ kind: "fold", key: `${f}:f${row.id}`, file: f, id: row.id, count: row.count });
      }
    }
  });
  return rows;
}

/** The highest line index a row shows, or undefined for a row that is no line. */
function lastIndex(row: Row): number | undefined {
  if (row.kind === "line") return row.index;
  if (row.kind !== "pair") return undefined;
  return Math.max(row.left?.index ?? -1, row.right?.index ?? -1);
}

/**
 * `rows` with the comment's editor after the row that shows the last selected line of file `file`
 * (or at the end of the file's rows when no row shows it, as when its lines are folded).
 */
export function withEditor(rows: readonly Row[], file: number, line: number): Row[] {
  let at = -1;
  for (let i = 0; i < rows.length; i += 1) {
    const row = rows[i];
    if (row?.file !== file) continue;
    at = i;
    const last = lastIndex(row);
    if (last !== undefined && last >= line) break;
  }
  if (at < 0) return [...rows];
  const editor: Row = { kind: "editor", key: `${file}:editor`, file };
  return [...rows.slice(0, at + 1), editor, ...rows.slice(at + 1)];
}

/** How tall a row is expected to be, for placing the sticky header; `chars` fit a line of code. */
export function guessHeight(row: Row, chars: number): number {
  const wrapped = (text: string): number =>
    Math.max(1, Math.ceil(text.length / chars)) * LINE_HEIGHT;
  switch (row.kind) {
    case "file":
      return HEADER_HEIGHT;
    case "fold":
    case "hunk":
      return FOLD_HEIGHT;
    case "editor":
      return EDITOR_HEIGHT;
    case "line":
      return wrapped(row.line.text);
    case "pair":
      return Math.max(wrapped(row.left?.line.text ?? ""), wrapped(row.right?.line.text ?? ""));
    case "note":
      return LINE_HEIGHT * 2;
  }
}

/** Where each row begins, and (last) where the list ends, from the guessed heights. */
export function offsets(rows: readonly Row[], chars: number): number[] {
  const out: number[] = [];
  let y = 0;
  for (const row of rows) {
    out.push(y);
    y += guessHeight(row, chars);
  }
  out.push(y);
  return out;
}

/** The file whose rows are at scroll position `y`. */
export function fileAt(
  rows: readonly Row[],
  starts: readonly number[],
  y: number,
): number | undefined {
  if (rows.length === 0) return undefined;
  let low = 0;
  let high = rows.length - 1;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if ((starts[mid] ?? 0) <= y) low = mid;
    else high = mid - 1;
  }
  return rows[low]?.file;
}

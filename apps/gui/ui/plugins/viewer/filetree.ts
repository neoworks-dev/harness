// The changed files as a tree of folders, for the viewer's file column: the rows to draw for the
// folders that are open, with a chain of folders that hold nothing else joined into one row
// (`src/lib/util`), and what each file did. Plain data, so tests reach it.
import type { DiffLine } from "@suite/protocol/types";
import { diffStats } from "../../lib/diff.ts";
import type { FileDiff } from "../../lib/unified.ts";

/** What a file's diff did to it. */
export type Status = "added" | "deleted" | "modified";

export const TREE_ROW_HEIGHT = 26;

/** Added when every line is, deleted when every line was, else modified. */
export function fileStatus(lines: readonly DiffLine[]): Status {
  const { added, removed } = diffStats(lines);
  if (added > 0 && removed === 0 && lines.every((line) => line.kind !== "context")) return "added";
  if (removed > 0 && added === 0 && lines.every((line) => line.kind !== "context"))
    return "deleted";
  return "modified";
}

interface Base {
  /** Names the row for the list. */
  key: string;
  /** The path: of the file, or of the folder (its last name is the one drawn). */
  path: string;
  /** What is drawn: a file's name, a folder's name or the names of the chain it joins. */
  name: string;
  /** How many folders are above the row. */
  depth: number;
  added: number;
  removed: number;
}

export type TreeRow =
  | (Base & { kind: "dir"; open: boolean; files: number })
  | (Base & { kind: "file"; status: Status; file: number });

interface Node {
  name: string;
  path: string;
  dirs: Map<string, Node>;
  /** Indexes into the files. */
  files: number[];
}

/** Names by code point, so the order does not depend on the locale. */
function order(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

function node(name: string, path: string): Node {
  return { name, path, dirs: new Map(), files: [] };
}

/** The rows of the tree of `files`, folders first and each by name, without the ones in `closed`. */
export function treeRows(files: readonly FileDiff[], closed: ReadonlySet<string>): TreeRow[] {
  const root = node("", "");
  const stats = files.map((file) => diffStats(file.lines));
  files.forEach((file, index) => {
    const parts = file.path.split("/");
    let at = root;
    for (let i = 0; i < parts.length - 1; i += 1) {
      const name = parts[i] ?? "";
      const path = parts.slice(0, i + 1).join("/");
      let next = at.dirs.get(name);
      if (next === undefined) {
        next = node(name, path);
        at.dirs.set(name, next);
      }
      at = next;
    }
    at.files.push(index);
  });

  /** Lines changed under a folder. */
  const sum = (dir: Node): { added: number; removed: number; files: number } => {
    let added = 0;
    let removed = 0;
    let count = 0;
    for (const index of dir.files) {
      added += stats[index]?.added ?? 0;
      removed += stats[index]?.removed ?? 0;
      count += 1;
    }
    for (const child of dir.dirs.values()) {
      const inner = sum(child);
      added += inner.added;
      removed += inner.removed;
      count += inner.files;
    }
    return { added, removed, files: count };
  };

  const rows: TreeRow[] = [];
  const walk = (dir: Node, depth: number): void => {
    const names = (map: Map<string, Node>): Node[] =>
      [...map.values()].sort((a, b) => order(a.name, b.name));
    for (const child of names(dir.dirs)) {
      // A folder with nothing but one folder joins it: `a` and `a/b` are one row, `a/b`.
      let shown = child;
      let name = child.name;
      while (shown.files.length === 0 && shown.dirs.size === 1) {
        const only = [...shown.dirs.values()][0] as Node;
        name = `${name}/${only.name}`;
        shown = only;
      }
      const total = sum(shown);
      const open = !closed.has(shown.path);
      rows.push({
        kind: "dir",
        key: `dir:${shown.path}`,
        path: shown.path,
        name,
        depth,
        open,
        added: total.added,
        removed: total.removed,
        files: total.files,
      });
      if (open) walk(shown, depth + 1);
    }
    const here = [...dir.files].sort((a, b) => {
      return order(files[a]?.path ?? "", files[b]?.path ?? "");
    });
    for (const index of here) {
      const file = files[index] as FileDiff;
      rows.push({
        kind: "file",
        key: `file:${file.path}`,
        path: file.path,
        name: file.path.slice(file.path.lastIndexOf("/") + 1),
        depth,
        added: stats[index]?.added ?? 0,
        removed: stats[index]?.removed ?? 0,
        status: fileStatus(file.lines),
        file: index,
      });
    }
  };
  walk(root, 0);
  return rows;
}

/** The paths of every folder row there is when all are open, for folding them all. */
export function folderPaths(files: readonly FileDiff[]): string[] {
  return treeRows(files, new Set())
    .filter((row) => row.kind === "dir")
    .map((row) => row.path);
}

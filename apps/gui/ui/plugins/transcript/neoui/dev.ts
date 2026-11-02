// The Dev components' reading of their props (docs/neoui.md): a file tree from paths, and the
// colour of each status.
import type { UiNode } from "@neoworks-dev/neo-protocol/types";
import type { Palette } from "../../../lib/palette.ts";
import { nodes, str } from "./props.ts";

/** A row of a FileTree: a folder (joined with a folder that is its only content) or a file. */
export type TreeRow =
  | { kind: "dir"; name: string; depth: number }
  | { kind: "file"; name: string; depth: number; file: UiNode };

interface Folder {
  dirs: Map<string, Folder>;
  files: UiNode[];
}

/** The files of a FileTree as rows: folders first, then files, each by name, like the terminal. */
export function treeRows(tree: UiNode): TreeRow[] {
  const root: Folder = { dirs: new Map(), files: [] };
  for (const file of nodes(tree, "files")) {
    const parts = (str(file, "path") ?? "").replace(/\/+$/, "").split("/");
    parts.pop();
    let node = root;
    for (const part of parts.filter((p) => p !== "")) {
      const name = `${part}/`;
      let next = node.dirs.get(name);
      if (!next) {
        next = { dirs: new Map(), files: [] };
        node.dirs.set(name, next);
      }
      node = next;
    }
    node.files.push(file);
  }
  const rows: TreeRow[] = [];
  const base = (file: UiNode): string => {
    const path = str(file, "path") ?? "";
    const trimmed = path.replace(/\/+$/, "");
    const name = trimmed.split("/").pop() ?? trimmed;
    return path.endsWith("/") ? `${name}/` : name;
  };
  const walk = (dir: Folder, depth: number): void => {
    for (const [first, start] of [...dir.dirs].sort(([a], [b]) => a.localeCompare(b))) {
      let name = first;
      let node = start;
      while (node.files.length === 0 && node.dirs.size === 1) {
        const [[next, only]] = [...node.dirs];
        name += next;
        node = only;
      }
      rows.push({ kind: "dir", name, depth });
      walk(node, depth + 1);
    }
    const files = [...dir.files].sort((a, b) =>
      base(a).toLowerCase().localeCompare(base(b).toLowerCase()),
    );
    for (const file of files) rows.push({ kind: "file", name: base(file), depth, file });
  };
  walk(root, 0);
  return rows;
}

/** A file's mark and its colour, as in the files panel. */
export function fileMark(status: string | undefined, p: Palette): { mark: string; color: string } {
  switch (status) {
    case "added":
      return { mark: "A", color: p.success };
    case "deleted":
      return { mark: "D", color: p.danger };
    case "renamed":
      return { mark: "R", color: p.accent };
    case "untracked":
      return { mark: "?", color: p.muted };
    case "unchanged":
      return { mark: "·", color: p.muted };
    default:
      return { mark: "M", color: p.accent };
  }
}

/** A test's, check's or run's status as a glyph and a colour. */
export function statusLook(
  status: string | undefined,
  p: Palette,
): { glyph: string; color: string } {
  switch (status) {
    case "passed":
      return { glyph: "✓", color: p.success };
    case "skipped":
      return { glyph: "○", color: p.muted };
    case "pending":
      return { glyph: "◐", color: p.accent };
    default:
      return { glyph: "✗", color: p.danger };
  }
}

/** A pull request's or issue's state as a label and a colour. */
export function stateLook(node: UiNode, p: Palette): { label: string; color: string } {
  switch (str(node, "state")) {
    case "draft":
      return { label: "Draft", color: p.muted };
    case "merged":
      return { label: "Merged", color: p.accent };
    case "closed":
      return node.component === "Issue"
        ? { label: "Closed", color: p.accent }
        : { label: "Closed", color: p.danger };
    default:
      return { label: "Open", color: p.success };
  }
}

/** `path:line`. */
export function place(node: UiNode): string {
  const path = str(node, "path") ?? "";
  const line = str(node, "line");
  return line === undefined ? path : `${path}:${line}`;
}

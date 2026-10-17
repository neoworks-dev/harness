import { expect, test } from "bun:test";
import type { DiffLine } from "@suite/protocol/types";
import type { FileDiff } from "../ui/lib/unified.ts";
import { fileStatus, folderPaths, treeRows } from "../ui/plugins/viewer/filetree.ts";
import { statBar } from "../ui/plugins/viewer/look.ts";

const add = (n: number): DiffLine => ({ kind: "add", text: `a${n}`, new: n });
const remove = (n: number): DiffLine => ({ kind: "remove", text: `r${n}`, old: n });
const context = (n: number): DiffLine => ({ kind: "context", text: `c${n}`, old: n, new: n });

const file = (path: string, lines: DiffLine[] = [add(1), remove(1)]): FileDiff => ({ path, lines });

/** The rows as `depth name` with a `/` for an open folder, for reading. */
function shape(files: FileDiff[], closed: string[] = []): string[] {
  return treeRows(files, new Set(closed)).map((row) => {
    const name = row.kind === "dir" ? `${row.name}/` : row.name;
    return `${" ".repeat(row.depth)}${name}`;
  });
}

test("folders come first and every level is sorted by name", () => {
  const files = [
    file("z.md"),
    file("src/b.ts"),
    file("a.txt"),
    file("src/a.ts"),
    file("docs/x.md"),
  ];
  expect(shape(files)).toEqual(["docs/", " x.md", "src/", " a.ts", " b.ts", "a.txt", "z.md"]);
});

test("a folder that holds only a folder joins it into one row", () => {
  const files = [file("crates/core/src/lib.rs"), file("crates/core/src/engine.rs")];
  expect(shape(files)).toEqual(["crates/core/src/", " engine.rs", " lib.rs"]);
  // Once something else lives in the outer folder, they part.
  expect(shape([...files, file("crates/Cargo.toml")])).toEqual([
    "crates/",
    " core/src/",
    "  engine.rs",
    "  lib.rs",
    " Cargo.toml",
  ]);
});

test("a folded folder leaves out what is in it", () => {
  const files = [file("src/a.ts"), file("src/lib/b.ts"), file("top.ts")];
  expect(shape(files, ["src/lib"])).toEqual(["src/", " lib/", " a.ts", "top.ts"]);
  expect(shape(files, ["src"])).toEqual(["src/", "top.ts"]);
  const rows = treeRows(files, new Set(["src"]));
  expect(rows[0]).toMatchObject({ kind: "dir", open: false, files: 2, path: "src" });
});

test("a folder adds up what is under it", () => {
  const files = [file("src/a.ts", [add(1), add(2)]), file("src/b/c.ts", [remove(1)])];
  const dir = treeRows(files, new Set())[0];
  expect(dir).toMatchObject({ kind: "dir", added: 2, removed: 1, files: 2 });
  const leaf = treeRows(files, new Set()).find((row) => row.path === "src/a.ts");
  expect(leaf).toMatchObject({
    kind: "file",
    added: 2,
    removed: 0,
    file: 0,
    depth: 1,
    name: "a.ts",
  });
});

test("the folders to fold are every one that shows", () => {
  const files = [file("a/b/c.ts"), file("a/d.ts"), file("e/f/g.ts"), file("h.ts")];
  expect(folderPaths(files)).toEqual(["a", "a/b", "e/f"]);
  expect(folderPaths([file("x.ts")])).toEqual([]);
});

test("a file is new, deleted or changed by its lines", () => {
  expect(fileStatus([add(1), add(2)])).toBe("added");
  expect(fileStatus([remove(1), remove(2)])).toBe("deleted");
  expect(fileStatus([context(1), add(2)])).toBe("modified");
  expect(fileStatus([remove(1), add(1)])).toBe("modified");
  expect(fileStatus([])).toBe("modified");
});

test("the diffstat bar shares five squares between added and removed lines", () => {
  expect(statBar(10, 0)).toEqual(["add", "add", "add", "add", "add"]);
  expect(statBar(0, 7)).toEqual(["remove", "remove", "remove", "remove", "remove"]);
  expect(statBar(5, 5)).toEqual(["add", "add", "add", "remove", "remove"]);
  expect(statBar(1, 99)).toEqual(["add", "remove", "remove", "remove", "remove"]);
  expect(statBar(99, 1)).toEqual(["add", "add", "add", "add", "remove"]);
  expect(statBar(0, 0)).toEqual(["none", "none", "none", "none", "none"]);
});

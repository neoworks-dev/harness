import { expect, test } from "bun:test";
import { parseUnified } from "../ui/lib/unified.ts";

const TWO_FILES = [
  "diff --git a/src/header.css b/src/header.css",
  "index 1..2 100644",
  "--- a/src/header.css",
  "+++ b/src/header.css",
  "@@ -1,3 +1,4 @@",
  " header {",
  "-  position: fixed;",
  "+  position: sticky;",
  "+  scrollbar-gutter: stable;",
  " }",
  "@@ -20,2 +21,2 @@ later",
  "",
  "-a",
  "+b",
  "diff --git a/old.txt b/old.txt",
  "deleted file mode 100644",
  "--- a/old.txt",
  "+++ /dev/null",
  "@@ -1 +0,0 @@",
  "-bye",
  "\\ No newline at end of file",
  "",
].join("\n");

test("a unified diff becomes numbered lines per file", () => {
  const files = parseUnified(TWO_FILES);
  expect(files.map((f) => f.path)).toEqual(["src/header.css", "old.txt"]);
  const [css] = files;
  expect(css?.lines.map((l) => `${l.kind}:${l.old ?? ""}:${l.new ?? ""}:${l.text}`)).toEqual([
    "hunk:::@@ -1,3 +1,4 @@",
    "context:1:1:header {",
    "remove:2::  position: fixed;",
    "add::2:  position: sticky;",
    "add::3:  scrollbar-gutter: stable;",
    "context:3:4:}",
    "hunk:::@@ -20,2 +21,2 @@ later",
    // Some tools strip the space of an empty context line.
    "context:20:21:",
    "remove:21::a",
    "add::22:b",
  ]);
  expect(files[1]?.lines.map((l) => l.kind)).toEqual(["hunk", "remove"]);
});

test("a no-index diff of a new file has its path from the header lines", () => {
  const text =
    "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";
  const [file] = parseUnified(text);
  expect(file?.path).toBe("new.txt");
  expect(file?.lines.map((l) => l.new)).toEqual([undefined, 1, 2]);
  expect(parseUnified("")).toEqual([]);
  expect(parseUnified("no diff here\n")).toEqual([]);
});

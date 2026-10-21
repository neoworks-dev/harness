// A unified diff (`git diff`, `gh pr diff`) as the `DiffLine`s the diff view draws. Several files
// become several parts, each with its path.
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";

export interface FileDiff {
  /** The new path (the old one for a deletion). */
  path: string;
  lines: DiffLine[];
}

/** The new path, else the old one (a deletion has no new one). */
function newestPath(path: string | undefined, oldPath: string | undefined): string {
  if (path !== undefined) return path;
  return oldPath ?? "";
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

function pathOf(line: string, prefix: "---" | "+++"): string | undefined {
  const raw = line.slice(prefix.length).trim().replace(/\t.*$/, "");
  if (raw === "/dev/null") return undefined;
  return raw.replace(/^[ab]\//, "");
}

/** Splits a unified diff into files and numbers the lines of each hunk. Anything unknown is skipped. */
export function parseUnified(text: string): FileDiff[] {
  const files: FileDiff[] = [];
  let file: FileDiff | undefined;
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  let oldPath: string | undefined;
  const lines = text.split("\n");
  // The newline ending the last line is not an empty context line.
  if (lines[lines.length - 1] === "") lines.pop();
  for (const line of lines) {
    if (line.startsWith("diff --git ")) {
      const match = /^diff --git a\/(.*?) b\/(.*)$/.exec(line);
      file = { path: match?.[2] ?? "", lines: [] };
      files.push(file);
      inHunk = false;
      oldPath = undefined;
      continue;
    }
    if (!inHunk && line.startsWith("--- ")) {
      oldPath = pathOf(line, "---");
      if (file && file.path === "" && oldPath) file.path = oldPath;
      continue;
    }
    if (!inHunk && line.startsWith("+++ ")) {
      const path = pathOf(line, "+++");
      // `git diff --no-index` has no `diff --git` header we can rely on for the path.
      if (!file) {
        file = { path: newestPath(path, oldPath), lines: [] };
        files.push(file);
      } else if (path) file.path = path;
      continue;
    }
    const hunk = HUNK.exec(line);
    if (hunk) {
      if (!file) {
        file = { path: "", lines: [] };
        files.push(file);
      }
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      inHunk = true;
      file.lines.push({ kind: "hunk", text: line });
      continue;
    }
    if (!file || !inHunk) continue;
    if (line.startsWith("+")) file.lines.push({ kind: "add", text: line.slice(1), new: newNo++ });
    else if (line.startsWith("-"))
      file.lines.push({ kind: "remove", text: line.slice(1), old: oldNo++ });
    else if (line.startsWith(" ") || line === "") {
      // Some tools strip the space of an empty context line.
      file.lines.push({ kind: "context", text: line.slice(1), old: oldNo++, new: newNo++ });
    }
    // "\\ No newline at end of file" carries nothing.
  }
  return files.filter((f) => f.lines.length > 0);
}

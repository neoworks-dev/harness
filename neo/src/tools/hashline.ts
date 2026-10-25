// Hashline anchors (after github.com/kebbbnnn/hashline): read tags each line with its number and
// a two-character hash of its text, `12:k3|const a = 1;`, and edit names the lines it changes by
// those tags instead of repeating their text. The number says where; the hash checks the line is
// still what the model read. Two characters are 1296 values, so hashes repeat within a file: a
// line that moved is only followed when its hash appears once near where it was.
import { createHash } from "node:crypto";

/** Lines a moved anchor is looked for on either side of where it was. */
const MOVE_WINDOW = 30;

/** Lines shown on either side of a stale anchor, so the model can retry without reading again. */
const CONTEXT = 3;

const ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz";

/** The line's hash: md5 of the text without leading and trailing whitespace, as two characters. */
export function lineHash(line: string): string {
  const digest = createHash("md5").update(line.trim()).digest();
  const value = digest.readUInt16BE(0) % (ALPHABET.length * ALPHABET.length);
  return (
    (ALPHABET[Math.floor(value / ALPHABET.length)] ?? "0") +
    (ALPHABET[value % ALPHABET.length] ?? "0")
  );
}

/** `12:k3|text` for each line, numbered from `first`. */
export function tagLines(lines: readonly string[], first = 1): string[] {
  return lines.map((line, i) => `${first + i}:${lineHash(line)}|${line}`);
}

/** A file's lines; a final newline ends the last line rather than starting an empty one. */
export function fileLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

export interface Anchor {
  /** Unset when only the hash was given. */
  line?: number;
  hash: string;
}

/**
 * `12:k3`, or just `k3`; what follows a `|` (the line's text, copied along) is ignored. Throws
 * with what was wrong, naming `label`.
 */
export function parseAnchor(value: string, label: string): Anchor {
  const tag = value.split("|")[0]?.trim() ?? "";
  const match = /^(?:(\d+):)?([0-9a-z]{2})$/.exec(tag);
  if (!match) {
    throw new Error(`${label}: "${value}" is not a line tag like "12:k3" (from read).`);
  }
  const hash = match[2] ?? "";
  return match[1] === undefined ? { hash } : { line: Number(match[1]), hash };
}

/** Where an anchor points now: a 0-based index, and how far it moved (0: not at all). */
export interface Resolved {
  index: number;
  moved: number;
}

/** The lines around `line` (1-based) as read shows them. */
function around(lines: readonly string[], line: number): string {
  const from = Math.max(1, line - CONTEXT);
  const to = Math.min(lines.length, line + CONTEXT);
  if (from > to) return "(the file is empty)";
  return tagLines(lines.slice(from - 1, to), from).join("\n");
}

/**
 * Finds the line an anchor names: the line at its number when the hash matches, else the one
 * line with that hash within MOVE_WINDOW lines of it. Throws, with the current lines there,
 * when there is none or more than one.
 */
export function resolveAnchor(lines: readonly string[], anchor: Anchor, label: string): Resolved {
  if (anchor.line === undefined) return resolveHash(lines, anchor.hash, label);
  const at = anchor.line - 1;
  const current = lines[at];
  if (current !== undefined && lineHash(current) === anchor.hash) return { index: at, moved: 0 };
  const found: number[] = [];
  const from = Math.max(0, at - MOVE_WINDOW);
  const to = Math.min(lines.length - 1, at + MOVE_WINDOW);
  for (let i = from; i <= to; i++) {
    if (lineHash(lines[i] ?? "") === anchor.hash) found.push(i);
  }
  const [only] = found;
  if (found.length === 1 && only !== undefined) return { index: only, moved: only - at };
  const tag = `${anchor.line}:${anchor.hash}`;
  let why = `line ${anchor.line} changed since it was read (${tag} not found near it)`;
  if (found.length > 1) {
    why = `${tag} no longer matches line ${anchor.line}, and ${found.length} lines near it have that hash`;
  } else if (anchor.line > lines.length) {
    why = `${tag} is past the end of the file (${lines.length} lines)`;
  }
  const shown = Math.min(Math.max(anchor.line, 1), Math.max(lines.length, 1));
  throw new Error(`${label}: ${why}. The lines there now:\n${around(lines, shown)}`);
}

/** A hash without its line number: fine when one line has it, else the lines that do. */
function resolveHash(lines: readonly string[], hash: string, label: string): Resolved {
  const found: number[] = [];
  lines.forEach((line, i) => {
    if (lineHash(line) === hash) found.push(i);
  });
  const [only] = found;
  if (found.length === 1 && only !== undefined) return { index: only, moved: 0 };
  if (found.length === 0) {
    throw new Error(`${label}: no line has the hash ${hash}; read the file again for its tags.`);
  }
  const tags = found.slice(0, 5).map((i) => tagLines([lines[i] ?? ""], i + 1).join(""));
  throw new Error(
    `${label}: ${found.length} lines have the hash ${hash}; give the line number too, ` +
      `as in "${(found[0] ?? 0) + 1}:${hash}":\n${tags.join("\n")}`,
  );
}

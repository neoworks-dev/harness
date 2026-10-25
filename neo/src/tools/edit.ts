import { readFile, writeFile } from "node:fs/promises";
import type { AgentTool } from "../loop/index.ts";
import { Type } from "typebox";
import type { PermissionAsk, RequestPermission } from "../agent.ts";
import { fileLines, parseAnchor, resolveAnchor, tagLines } from "./hashline.ts";
import { resolveWritablePath } from "./paths.ts";
import { askToProceed, reviewEdit, withNote } from "./permission.ts";
import type { FileChangeDetails } from "./write.ts";

const parameters = Type.Object({
  path: Type.String({
    description: "File to edit: absolute, relative to the working directory, or starting with ~.",
  }),
  edits: Type.Array(
    Type.Object({
      start: Type.Optional(
        Type.String({
          description: 'First line to replace, by its tag from read: "12:k3".',
        }),
      ),
      end: Type.Optional(
        Type.String({
          description: "Last line to replace (inclusive), by its tag. Default: start (one line).",
        }),
      ),
      after: Type.Optional(
        Type.String({
          description:
            'Insert after this line instead of replacing, by its tag; "0" inserts at the top.',
        }),
      ),
      newText: Type.String({
        description:
          "The new lines, without the tags and without a final newline. Empty with start: " +
          "deletes the lines.",
      }),
    }),
    {
      description:
        "Line edits, each with start (and end) to replace lines or after to insert. All tags " +
        "are from the file as it was before this call.",
    },
  ),
});

export type Edit = { start?: string; end?: string; after?: string; newText: string };

const BOM = "\uFEFF";

/** Lines of context shown around each changed region in the result. */
const RESULT_CONTEXT = 2;
/** At most this many lines in the result; the rest is for read. */
const RESULT_LINES = 80;

/** One edit as a splice of the original lines: replace `count` lines at `index`. */
export interface Splice {
  i: number;
  index: number;
  count: number;
  lines: string[];
}

function newLines(text: string): string[] {
  const normalized = text.replaceAll("\r\n", "\n").replace(/\n$/, "");
  const lines: string[] = [];
  if (normalized !== "") lines.push(...normalized.split("\n"));
  return lines;
}

/** Where an edit goes in the original lines, and the notes on lines that had moved. */
export function spliceOf(edit: Edit, i: number, lines: readonly string[], moves: string[]): Splice {
  const label = `edits[${i}]`;
  const replacing = edit.start !== undefined;
  if (replacing === (edit.after !== undefined)) {
    throw new Error(`${label}: give either start (to replace lines) or after (to insert).`);
  }
  if (edit.after !== undefined) {
    if (edit.end !== undefined) throw new Error(`${label}: end goes with start, not after.`);
    if (/^0:?$/.test(edit.after.trim()))
      return { i, index: 0, count: 0, lines: newLines(edit.newText) };
    const anchor = parseAnchor(edit.after, `${label}.after`);
    const at = resolveAnchor(lines, anchor, `${label}.after`);
    if (at.moved !== 0) moves.push(`${label}: line ${anchor.line} had moved to ${at.index + 1}.`);
    return { i, index: at.index + 1, count: 0, lines: newLines(edit.newText) };
  }
  const startAnchor = parseAnchor(edit.start ?? "", `${label}.start`);
  const start = resolveAnchor(lines, startAnchor, `${label}.start`);
  let endIndex = start.index;
  if (edit.end !== undefined) {
    const endAnchor = parseAnchor(edit.end, `${label}.end`);
    const end = resolveAnchor(lines, endAnchor, `${label}.end`);
    // A range moves as a whole; ends that moved apart point at something else.
    if (end.moved !== start.moved) {
      throw new Error(
        `${label}: lines ${edit.start}-${edit.end} changed since they were read; ` +
          "read them again.",
      );
    }
    endIndex = end.index;
    if (endIndex < start.index) {
      throw new Error(`${label}: end (${edit.end}) is before start (${edit.start}).`);
    }
  }
  if (start.moved !== 0) {
    moves.push(`${label}: line ${startAnchor.line} had moved to ${start.index + 1}.`);
  }
  return {
    i,
    index: start.index,
    count: endIndex - start.index + 1,
    lines: newLines(edit.newText),
  };
}

/** In file order; at the same line an insert comes before a replacement, else edit order. */
function byPosition(a: Splice, b: Splice): number {
  return a.index - b.index || Number(a.count > 0) - Number(b.count > 0) || a.i - b.i;
}

/** The changed regions of the new file with their tags (and a little context), merged. */
function changedRegions(splices: readonly Splice[], lines: readonly string[]): string {
  // Where each splice landed in the new file: applied bottom-up, so earlier ones shift later.
  const ranges: [number, number][] = [];
  let shift = 0;
  for (const s of [...splices].sort(byPosition)) {
    const from = s.index + shift;
    ranges.push([from, from + s.lines.length]);
    shift += s.lines.length - s.count;
  }
  const merged: [number, number][] = [];
  for (const [from, to] of ranges) {
    const a = Math.max(0, from - RESULT_CONTEXT);
    const b = Math.min(lines.length, to + RESULT_CONTEXT);
    const last = merged.at(-1);
    if (last && a <= last[1]) last[1] = Math.max(last[1], b);
    else merged.push([a, b]);
  }
  const parts: string[] = [];
  let shown = 0;
  for (const [from, to] of merged) {
    if (shown >= RESULT_LINES) {
      parts.push("… more changes below; read the file for them.");
      break;
    }
    const end = Math.min(to, from + RESULT_LINES - shown);
    parts.push(tagLines(lines.slice(from, end), from + 1).join("\n"));
    shown += end - from;
  }
  return parts.join("\n…\n");
}

export function createEditTool(
  cwd: string,
  ask?: RequestPermission,
): AgentTool<typeof parameters, FileChangeDetails> {
  return {
    name: "edit",
    label: "edit",
    description:
      'Edit a file by line tags from read (`12:k3|…` → "12:k3"): replace lines start..end ' +
      "with newText, or insert newText after a line. All edits in a call use the tags of the " +
      "file as it was before the call and must not overlap. A tag whose line changed fails " +
      "with the current lines, tags included, so you can retry without reading again. The " +
      "result shows the changed lines with their new tags. Line endings (CRLF) and a UTF-8 " +
      "BOM are preserved.",
    parameters,
    async execute(toolCallId, { path, edits }, signal) {
      if (edits.length === 0) throw new Error("edits is empty.");
      const absolute = resolveWritablePath(cwd, path);
      const raw = await readFile(absolute, "utf8");
      const hasBom = raw.startsWith(BOM);
      const crlf = raw.includes("\r\n");
      const original = raw.slice(hasBom ? 1 : 0).replaceAll("\r\n", "\n");
      const finalNewline = original === "" || original.endsWith("\n");
      const lines = fileLines(original);

      const moves: string[] = [];
      const splices = edits.map((edit, i) => spliceOf(edit, i, lines, moves));
      const ordered = [...splices].sort(byPosition);
      for (let k = 1; k < ordered.length; k++) {
        const prev = ordered[k - 1];
        const next = ordered[k];
        if (prev && next && next.index < prev.index + prev.count) {
          throw new Error(
            `edits[${prev.i}] and edits[${next.i}] overlap; merge them into one edit.`,
          );
        }
      }

      const result = [...lines];
      for (const s of [...ordered].reverse()) result.splice(s.index, s.count, ...s.lines);
      const joined = result.join("\n");
      let updated = joined;
      if (joined !== "" && finalNewline) updated += "\n";
      if (updated === original) throw new Error(`The edits make no change to ${path}.`);

      const eol = crlf ? updated.replaceAll("\n", "\r\n") : updated;
      const request: PermissionAsk = {
        title: `edit ${path}`,
        kind: "edit",
        toolCallId,
        content: [{ type: "diff", path: absolute, oldText: original, newText: updated }],
      };
      const proceed = await askToProceed(ask, request, "The user declined this edit.", signal);
      await writeFile(absolute, hasBom ? BOM + eol : eol);
      const note = await reviewEdit(ask, proceed, request, signal);
      const summary = [`Applied ${edits.length} edit(s) to ${path}.`, ...moves];
      const regions = changedRegions(splices, result);
      if (regions) summary.push("", regions);
      return {
        content: [{ type: "text", text: withNote(summary.join("\n"), note) }],
        details: { path: absolute, oldText: original, newText: updated },
      };
    },
  };
}

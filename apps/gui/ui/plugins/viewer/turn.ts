// The files the agent edited in the latest turn, from the diffs its tool calls carried (the old and
// the new text of a file), as the viewer's "Latest turn" scope. Plain functions over the session's
// events, so tests reach them.
import type { SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import { relativize } from "../../lib/git-status.ts";
import type { FileDiff } from "../../lib/unified.ts";
import { lineDiff } from "./linediff.ts";

/** One edit of a file, as a tool call reported it. */
export interface TurnEdit {
  /** Relative to the session's directory when inside it. */
  path: string;
  /** Absent when the edit created the file. */
  oldText?: string;
  newText: string;
}

/** What one tool call changed. A later update of the call replaces it. */
interface Call {
  id: string;
  edits: readonly TurnEdit[];
}

export interface Turn {
  calls: readonly Call[];
  /** Bumped when an edit comes in and when a turn ends: the moments git is worth asking again. */
  version: number;
}

export const NO_TURN: Turn = { calls: [], version: 0 };

/** The turn after one more event of the session in `cwd`: a message starts a turn, an edit joins it. */
export function applyTurn(state: Turn, record: SessionEventRecord, cwd: string): Turn {
  const { event } = record;
  if (event.type === "user_message") return { calls: [], version: state.version + 1 };
  if (event.type === "turn_ended") return { ...state, version: state.version + 1 };
  if (event.type !== "tool_call_update") return state;
  const edits: TurnEdit[] = [];
  for (const content of event.content ?? []) {
    if (content.type !== "diff" || content.path === "") continue;
    const edit: TurnEdit = { path: relativize(cwd, content.path), newText: content.newText };
    if (content.oldText !== undefined) edit.oldText = content.oldText;
    edits.push(edit);
  }
  if (edits.length === 0) return state;
  const call = { id: event.toolCallId, edits };
  const at = state.calls.findIndex((other) => other.id === call.id);
  const calls = at < 0 ? [...state.calls, call] : state.calls.map((c, i) => (i === at ? call : c));
  return { calls, version: state.version + 1 };
}

/** Every edit of the turn, in the order the calls came. */
export function turnEdits(turn: Turn): TurnEdit[] {
  return turn.calls.flatMap((call) => call.edits);
}

/** Edits of one file in a row, where each starts from what the last left, as one edit. */
function chained(edits: readonly TurnEdit[]): TurnEdit[] {
  const out: TurnEdit[] = [];
  for (const edit of edits) {
    const last = out[out.length - 1];
    if (last !== undefined && last.newText === edit.oldText) {
      out[out.length - 1] = { ...last, newText: edit.newText };
    } else {
      out.push({ ...edit });
    }
  }
  return out;
}

/** The last diffs made, so an edit coming in does not diff the earlier ones again. */
const made = new Map<string, FileDiff["lines"]>();
const MADE = 24;

function diffOf(edit: TurnEdit): FileDiff["lines"] {
  const id = `${edit.oldText ?? "\0none"}\0${edit.newText}`;
  const known = made.get(id);
  if (known !== undefined) return known;
  const lines = lineDiff(edit.oldText, edit.newText);
  if (made.size >= MADE) made.delete(made.keys().next().value ?? "");
  made.set(id, lines);
  return lines;
}

/** The turn's edits as one diff per file, left out where the edits cancel. Files by first edit. */
export function turnDiffs(edits: readonly TurnEdit[]): FileDiff[] {
  const paths = [...new Set(edits.map((edit) => edit.path))];
  const files: FileDiff[] = [];
  for (const path of paths) {
    for (const edit of chained(edits.filter((other) => other.path === path))) {
      const lines = diffOf(edit);
      if (lines.some((line) => line.kind !== "context")) files.push({ path, lines });
    }
  }
  return files;
}

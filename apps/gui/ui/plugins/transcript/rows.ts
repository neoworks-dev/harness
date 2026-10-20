// The chat's items as the rows the list draws. A turn is a user's message and what the agent did
// for it. While it runs, its items show as they come, consecutive tool calls merged into groups.
// When it is done, everything between the message and the agent's last reply folds under a
// "Worked for 3m 43s" row, closed unless opened; a turn that failed or was stopped never folds,
// and the last reply comes with the files the turn changed. Plain functions, tested without a
// window.
import type { Block, ViewItem } from "@suite/protocol/types";
import { duration } from "./format.ts";
import type { Timeline } from "./timeline.ts";
import { changedFiles, classify, isPlanCall, type FileChange, type WorkEntry } from "./work.ts";

export type Row =
  /** An item drawn by `chat.item`: the user's message, a notice. */
  | { kind: "item"; key: string; item: ViewItem }
  /**
   * The agent's reply; the last of a finished turn is `final`, with the files the turn changed.
   * `split`: its text is drawn by the `prose` rows before it, and this row is what goes under it.
   */
  | {
      kind: "message";
      key: string;
      item: ViewItem;
      final: boolean;
      changes: FileChange[];
      at?: number;
      split?: boolean;
    }
  /** One top-level block of a reply's text (splitMessages), so the list builds only those in view. */
  | { kind: "prose"; key: string; ref: string; block: Block; first: boolean; label?: string }
  /** Consecutive tool calls (and thoughts) of one turn. */
  | { kind: "work"; key: string; entries: WorkEntry[]; live: boolean }
  /** The header of a finished turn's work. */
  | { kind: "fold"; key: string; foldKey: string; open: boolean; label: string }
  /** The note that the user stopped a turn. */
  | { kind: "stopped"; key: string; label: string }
  /** The running turn's clock. */
  | { kind: "working"; key: string; startedAt?: number };

export type RowKind = Row["kind"];

export interface RowsInput {
  items: readonly ViewItem[];
  timeline: Timeline;
  /** The session's directory, which paths are shown relative to. */
  cwd: string;
  /** Whether a turn is running now; it is the last one. */
  running: boolean;
  /** When the running turn started. */
  startedAt?: number;
  /** Whether the fold of a turn is open. */
  isOpen: (foldKey: string) => boolean;
}

const NONE: FileChange[] = [];
const NO_ENTRIES: WorkEntry[] = [];

/** One item of a turn, as the rows will draw it. */
type Part =
  | { type: "message"; item: ViewItem }
  | { type: "notice"; item: ViewItem }
  | { type: "work"; item: ViewItem; entry: WorkEntry };

interface Turn {
  user?: ViewItem;
  body: ViewItem[];
}

function splitTurns(items: readonly ViewItem[]): Turn[] {
  const turns: Turn[] = [];
  for (const item of items) {
    if (item.kind === "user") turns.push({ user: item, body: [] });
    else {
      let turn = turns.at(-1);
      if (turn === undefined) {
        turn = { body: [] };
        turns.push(turn);
      }
      turn.body.push(item);
    }
  }
  return turns;
}

function partsOf(turn: Turn, active: boolean, input: RowsInput): Part[] {
  const { timeline, cwd } = input;
  const parts: Part[] = [];
  turn.body.forEach((item, index) => {
    if (item.kind === "agent") parts.push({ type: "message", item });
    else if (item.kind === "notice") parts.push({ type: "notice", item });
    else if (item.kind === "tool" || item.kind === "thought") {
      const facts = timeline.facts.get(item.ref);
      if (isPlanCall(item, facts)) return;
      const writing = active && index === turn.body.length - 1;
      const at = timeline.stamps.get(item.ref)?.first;
      parts.push({
        type: "work",
        item,
        entry: classify(item, { facts, cwd, active, writing, at }),
      });
    }
  });
  return parts;
}

/** The rows of parts that are drawn in place, tool calls merged into groups. */
function bodyRows(
  parts: readonly Part[],
  finalAt: number,
  changes: FileChange[],
  input: RowsInput,
  turn: Turn,
): Row[] {
  const { timeline } = input;
  const rows: Row[] = [];
  let group: WorkEntry[] = [];
  const flush = (): void => {
    const first = group[0];
    if (first === undefined) return;
    rows.push({
      kind: "work",
      key: `work:${first.id}`,
      entries: group,
      live: group.some((entry) => entry.live),
    });
    group = [];
  };
  const info = turn.user === undefined ? undefined : timeline.turns.get(turn.user.ref);
  parts.forEach((part, index) => {
    if (part.type === "work") {
      const alone = part.entry.kind === "question" || part.entry.kind === "plan";
      if (alone) flush();
      group.push(part.entry);
      if (alone) flush();
      return;
    }
    flush();
    if (part.type === "message") {
      const final = index === finalAt;
      rows.push({
        kind: "message",
        key: part.item.ref,
        item: part.item,
        final,
        changes: final ? changes : NONE,
        at: timeline.stamps.get(part.item.ref)?.last,
      });
    } else if (info?.noticeRef === part.item.ref && info.stopReason === "cancelled") {
      const took = (info.endedAt ?? info.startedAt) - info.startedAt;
      rows.push({
        kind: "stopped",
        key: `stopped:${part.item.ref}`,
        label: `You stopped after ${duration(took)}`,
      });
    } else {
      rows.push({ kind: "item", key: part.item.ref, item: part.item });
    }
  });
  flush();
  return rows;
}

/** The index of the last reply of a turn, or -1. */
function lastMessage(parts: readonly Part[]): number {
  for (let i = parts.length - 1; i >= 0; i -= 1) if (parts[i]?.type === "message") return i;
  return -1;
}

function entriesOf(parts: readonly Part[]): WorkEntry[] {
  const entries: WorkEntry[] = [];
  for (const part of parts) if (part.type === "work") entries.push(part.entry);
  return entries;
}

/** "Worked for 3m 43s": from the turn's first item to its last, as far as the events say. */
function workedLabel(turn: Turn, parts: readonly Part[], input: RowsInput): string {
  const { timeline } = input;
  const info = turn.user === undefined ? undefined : timeline.turns.get(turn.user.ref);
  if (info?.endedAt !== undefined) return `Worked for ${duration(info.endedAt - info.startedAt)}`;
  const first = turn.user === undefined ? undefined : timeline.stamps.get(turn.user.ref)?.first;
  const lastPart = parts.at(-1);
  const last = lastPart === undefined ? undefined : timeline.stamps.get(lastPart.item.ref)?.last;
  if (first === undefined || last === undefined || last < first) return "Worked";
  return `Worked for ${duration(last - first)}`;
}

function turnRows(turn: Turn, active: boolean, input: RowsInput): Row[] {
  const rows: Row[] = [];
  if (turn.user !== undefined) rows.push({ kind: "item", key: turn.user.ref, item: turn.user });
  const parts = partsOf(turn, active, input);
  const info = turn.user === undefined ? undefined : input.timeline.turns.get(turn.user.ref);
  // A finished turn's last reply stands out; while it runs nothing is last yet.
  const finalAt = active ? -1 : lastMessage(parts);
  const changes = changedFiles(finalAt < 0 ? NO_ENTRIES : entriesOf(parts));
  const failed =
    (info?.stopReason !== undefined && info.stopReason !== "end_turn") ||
    parts.some(
      (part) => part.type === "work" && part.entry.failed && part.entry.kind !== "question",
    );
  const cut = finalAt < 0 ? parts.length : finalAt;
  const foldable = !active && !failed;
  const hidden = parts.slice(0, foldable ? cut : 0).filter((part) => part.type !== "notice");
  if (hidden.length === 0 || turn.user === undefined) {
    return [...rows, ...bodyRows(parts, finalAt, changes, input, turn)];
  }
  const foldKey = `fold:${turn.user.ref}`;
  const open = input.isOpen(foldKey);
  rows.push({
    kind: "fold",
    key: foldKey,
    foldKey,
    open,
    label: workedLabel(turn, hidden, input),
  });
  // What stays in view: notices before the reply, then the reply and what follows it.
  const rest = parts.filter((part, index) => part.type === "notice" || index >= cut);
  const restFinal = rest.findIndex((part) => part === parts[finalAt]);
  if (open) rows.push(...bodyRows(hidden, -1, NONE, input, turn));
  rows.push(...bodyRows(rest, restFinal, changes, input, turn));
  return rows;
}

/** The rows to draw for the session's items. */
export function buildRows(input: RowsInput): Row[] {
  const turns = splitTurns(input.items);
  const rows: Row[] = [];
  turns.forEach((turn, index) => {
    rows.push(...turnRows(turn, input.running && index === turns.length - 1, input));
  });
  if (input.running) rows.push({ kind: "working", key: "working", startedAt: input.startedAt });
  return rows;
}

type ProseRow = Extract<Row, { kind: "prose" }>;

/** The prose row of each block, kept while the block is the same object (the daemon keeps them). */
const proseRows = new WeakMap<Block, ProseRow>();

function proseRow(item: ViewItem, block: Block, index: number): ProseRow {
  const key = `${item.ref}/${index}`;
  const first = index === 0;
  const label = first ? item.label : undefined;
  const known = proseRows.get(block);
  if (known && known.key === key && known.label === label) return known;
  const row: ProseRow = { kind: "prose", key, ref: item.ref, block, first };
  if (label !== undefined) row.label = label;
  proseRows.set(block, row);
  return row;
}

/**
 * Each reply of plain text (no role or tone) as one row per top-level block, then its own row for what goes under it. The
 * list builds only the rows near the window, and a long reply as a single row was laid out whole
 * on every frame while it streamed. Unchanged blocks keep their row objects.
 */
export function splitMessages(rows: readonly Row[]): Row[] {
  const out: Row[] = [];
  for (const row of rows) {
    const component = row.kind === "message" ? row.item.component : undefined;
    const plain = component?.type === "text" && component.role === undefined && !component.tone;
    if (row.kind !== "message" || component?.type !== "text" || !plain) {
      out.push(row);
      continue;
    }
    component.blocks.forEach((block, index) => out.push(proseRow(row.item, block, index)));
    out.push({ ...row, split: true });
  }
  return out;
}

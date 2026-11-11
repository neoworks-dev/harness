// The messages queued behind a running turn: how the composer lists them above the input and how
// the ones taken back (`unqueue`) go into the draft. Plain functions, so tests need no window.
import type { WaitingInput } from "@neoworks-dev/neo-protocol/types";

/** Rows the queue draws before it says how many more wait. */
export const QUEUE_ROWS = 3;

/** Whether the user queued it, from any app (any app may take it back). An agent's stays. */
export function isUsers(input: WaitingInput): boolean {
  return input.agent !== true;
}

/** The terminal's `[image 2]` markers, which stand for the images that come back as attachments. */
const MARKER = /\s*\[image \d+\]/g;

/** What the user wrote: the images are attached again, so their markers are dropped. */
function written(input: WaitingInput): string {
  const images = input.images?.length ?? 0;
  return images === 0 ? input.text : input.text.replace(MARKER, "").trim();
}

export interface QueueRow {
  /** `◷ first line`, with ` …` when more lines follow and `[image]` / `[N images]` for pictures. */
  label: string;
  /** The user's message: pressing it takes it back. An agent's cannot be taken back. */
  users: boolean;
}

export interface QueueView {
  rows: QueueRow[];
  /** Inputs past the rows shown; 0 when all fit. */
  more: number;
  /** Something in the queue is the user's, so Up (or a press) can edit it. */
  editable: boolean;
}

/** One input as its row. */
export function queueRow(input: WaitingInput): QueueRow {
  const lines = written(input)
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
  let label = `◷ ${lines[0] ?? ""}`.trimEnd();
  if (lines.length > 1) label += " …";
  const images = input.images?.length ?? 0;
  if (images === 1) label += "  [image]";
  else if (images > 1) label += `  [${images} images]`;
  return { label, users: isUsers(input) };
}

/**
 * The rows for `waiting` (oldest first): at most `limit`; when more wait, one fewer row and the
 * count of the rest, so the "+N more" line fits in the same space.
 */
export function queueView(waiting: readonly WaitingInput[], limit = QUEUE_ROWS): QueueView {
  const shown = waiting.length > limit ? limit - 1 : waiting.length;
  return {
    rows: waiting.slice(0, shown).map(queueRow),
    more: waiting.length - shown,
    editable: waiting.some(isUsers),
  };
}

/**
 * The draft after taking `inputs` back: their texts one per paragraph, in the order they were
 * queued, before whatever was typed meanwhile.
 */
export function redraftText(inputs: readonly WaitingInput[], typed: string): string {
  const parts = inputs.map((input) => written(input).trim());
  parts.push(typed.trim());
  return parts.filter((part) => part !== "").join("\n\n");
}

/** The images of the inputs taken back, in order, to attach again. */
export function redraftImages(inputs: readonly WaitingInput[]): string[] {
  return inputs.flatMap((input) => input.images ?? []);
}

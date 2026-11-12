// What a chat row is guessed to measure before it is drawn. The list counts a row as this until it
// has drawn it, and lays out every row it hasn't by it, so the closer the guess, the less the
// scrollbar moves as rows come into view. It only has to be close: the list measures every row it
// draws and keeps what is on screen in place when a row turns out taller or shorter.
import { blocksText, componentText } from "./components.ts";
import type { Row } from "./rows.ts";

// The numbers are what a long chat measured on average in the browser, not the styles' own.
/** A line of body text. */
const LINE = 22;
/** A line of a reply, with what its blocks add between them. */
const REPLY_LINE = 24;
/** An average character at the body size, for how many fit on a line. */
const CHAR = 7.5;
/** An average character of a reply, with its code and bold. */
const REPLY_CHAR = 8.5;
/** The bubble's padding on each side, and the header line under a user's message. */
const BUBBLE_PADDING = 12;
const USER_FRAME = 38;
/** A user's message longer than this many lines is cut to them until opened. */
const USER_LINES = 8;

/** How many lines `text` wraps to in `width` pixels, at `char` pixels a character. */
export function wrappedLines(text: string, width: number, char = CHAR): number {
  const perLine = Math.max(20, Math.floor(width / char));
  let count = 0;
  for (const line of text.split("\n")) count += Math.max(1, Math.ceil(line.length / perLine));
  return count;
}

/** The height `row` is guessed to come to in a column `width` pixels wide. */
export function estimateRow(row: Row, width: number): number {
  switch (row.kind) {
    case "item": {
      if (row.item.kind !== "user") return 40;
      const inside = width * 0.8 - BUBBLE_PADDING * 2;
      const lines = Math.min(wrappedLines(componentText(row.item.component), inside), USER_LINES);
      return USER_FRAME + BUBBLE_PADDING * 2 + lines * LINE;
    }
    case "prose":
      return 20 + wrappedLines(blocksText([row.block]), width, REPLY_CHAR) * REPLY_LINE;
    case "message": {
      const files = row.changes.length * 24;
      if (row.split) return 40 + files;
      return 20 + wrappedLines(componentText(row.item.component), width, REPLY_CHAR) * REPLY_LINE + files;
    }
    case "work":
      // A group of tool calls shows as one line until it is opened.
      return 36;
    default:
      return 40;
  }
}

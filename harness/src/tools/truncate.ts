export const MAX_LINES = 2000;
export const MAX_BYTES = 50 * 1024;

export interface Truncated {
  text: string;
  truncated: boolean;
  /** Lines in the input. */
  totalLines: number;
  /** Lines in `text` (a partial line counts as one). */
  shownLines: number;
}

/** A trailing newline ends the last line rather than starting a new one. */
export function splitLines(text: string): string[] {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** Last `maxBytes` bytes of `text`, never cutting a UTF-8 character in half. */
export function tailBytes(text: string, maxBytes: number): string {
  const buf = Buffer.from(text);
  if (buf.length <= maxBytes) return text;
  let start = buf.length - maxBytes;
  while (start < buf.length && ((buf[start] ?? 0) & 0xc0) === 0x80) start++;
  return buf.subarray(start).toString("utf8");
}

/** Keeps the first lines. Never returns a partial line; `shownLines` is 0 if line 1 is too big. */
export function truncateHead(text: string): Truncated {
  const lines = splitLines(text);
  const kept: string[] = [];
  let bytes = 0;
  for (const line of lines) {
    const size = Buffer.byteLength(line) + 1;
    if (kept.length >= MAX_LINES || bytes + size > MAX_BYTES) break;
    kept.push(line);
    bytes += size;
  }
  const truncated = kept.length < lines.length;
  if (!truncated) return { text, truncated, totalLines: lines.length, shownLines: lines.length };
  return {
    text: kept.join("\n"),
    truncated,
    totalLines: lines.length,
    shownLines: kept.length,
  };
}

/** Keeps the last lines. A single oversized last line is cut from its start. */
export function truncateTail(text: string): Truncated {
  const lines = splitLines(text);
  const kept: string[] = [];
  let bytes = 0;
  let cutLine = false;
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i] ?? "";
    const size = Buffer.byteLength(line) + 1;
    if (kept.length >= MAX_LINES) break;
    if (bytes + size > MAX_BYTES) {
      if (kept.length === 0) {
        kept.push(tailBytes(line, MAX_BYTES));
        cutLine = true;
      }
      break;
    }
    kept.unshift(line);
    bytes += size;
  }
  const truncated = cutLine || kept.length < lines.length;
  if (!truncated) return { text, truncated, totalLines: lines.length, shownLines: lines.length };
  return {
    text: kept.join("\n"),
    truncated,
    totalLines: lines.length,
    shownLines: kept.length,
  };
}

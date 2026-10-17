// What changed inside a line that was rewritten: the stretch of a removed line that its added
// counterpart no longer has, or the other way round, so the eye goes to the words that differ. Plain
// functions, so tests reach them.
import type { DiffLine } from "@suite/protocol/types";
import type { Token } from "./syntax-runs.ts";

/** A part of a line: characters `from` up to `to`. */
export interface Span {
  from: number;
  to: number;
}

/** A run of a line that may be marked as part of what changed. */
export interface Run extends Token {
  mark?: boolean;
}

/** More of a line than this changed and marking it says nothing. */
const MOST = 0.75;

const word = (char: string | undefined): boolean => char !== undefined && /\w/.test(char);

/**
 * The part of `text` that `other` does not have around it: what is left when their common start
 * and common end are taken off, widened to whole words. Undefined when nothing of `text` is left
 * (it only lost something) or when most of it is different.
 */
export function changedSpan(text: string, other: string): Span | undefined {
  let head = 0;
  while (head < text.length && head < other.length && text[head] === other[head]) head += 1;
  let tail = 0;
  while (
    tail < text.length - head &&
    tail < other.length - head &&
    text[text.length - 1 - tail] === other[other.length - 1 - tail]
  ) {
    tail += 1;
  }
  let from = head;
  let to = text.length - tail;
  if (to <= from) return undefined;
  while (from > 0 && word(text[from - 1]) && word(text[from])) from -= 1;
  while (to < text.length && word(text[to - 1]) && word(text[to])) to += 1;
  if (to - from > text.length * MOST) return undefined;
  return { from, to };
}

/** `tokens` cut at the ends of `span`, the part inside marked. */
export function markSpan(tokens: readonly Token[], span: Span): Run[] {
  const out: Run[] = [];
  let at = 0;
  for (const token of tokens) {
    const end = at + token.text.length;
    if (end <= span.from || at >= span.to) {
      out.push(token);
    } else {
      const from = Math.max(span.from, at) - at;
      const to = Math.min(span.to, end) - at;
      if (from > 0) out.push({ ...token, text: token.text.slice(0, from) });
      out.push({ ...token, text: token.text.slice(from, to), mark: true });
      if (to < token.text.length) out.push({ ...token, text: token.text.slice(to) });
    }
    at = end;
  }
  return out;
}

/** The runs of a line with what changed in it marked, when there is a counterpart to compare with. */
export function withMarks(
  text: string,
  tokens: readonly Token[] | undefined,
  partner: string | undefined,
): readonly Run[] | undefined {
  if (partner === undefined) return tokens;
  const span = changedSpan(text, partner);
  if (span === undefined) return tokens;
  return markSpan(tokens ?? [{ text }], span);
}

/**
 * For each removed line followed by added ones, the text of the line it was rewritten as, and the
 * other way round: the first removed with the first added, the second with the second.
 */
export function partners(lines: readonly DiffLine[]): Map<number, string> {
  const out = new Map<number, string>();
  let at = 0;
  while (at < lines.length) {
    if (lines[at]?.kind !== "remove") {
      at += 1;
      continue;
    }
    let mid = at;
    while (lines[mid]?.kind === "remove") mid += 1;
    let end = mid;
    while (lines[end]?.kind === "add") end += 1;
    const pairs = Math.min(mid - at, end - mid);
    for (let i = 0; i < pairs; i += 1) {
      out.set(at + i, lines[mid + i]?.text ?? "");
      out.set(mid + i, lines[at + i]?.text ?? "");
    }
    at = end;
  }
  return out;
}

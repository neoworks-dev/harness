// Parsed markdown as a flat list of rows. Nesting (a list in a list item, a quote around a
// list) becomes numbers on each row, so the chat draws it without nested components: the host
// runs the app on a small JavaScript stack, and every level of component nesting spends a lot of it.
import type { Block, ListBlock, Span, TableBlock, UiDoc } from "@neoworks-dev/neo-protocol/types";

/** How far a row sits below the previous one. */
export type RowGap = "none" | "tight" | "loose";

/** A list item's marker, drawn in the columns [at, at + width) in front of the item's first line. */
export interface Marker {
  text: string;
  at: number;
  width: number;
  /** A ticked task. */
  done: boolean;
}

interface RowBase {
  /** Characters before the text starts: the markers of the enclosing list items. */
  indent: number;
  markers: Marker[];
  /** How many quotes enclose the row: each gets a bar down the left. */
  quote: number;
  gap: RowGap;
}

export type MarkdownRow = RowBase &
  (
    | { kind: "paragraph"; spans: readonly Span[] }
    | { kind: "heading"; level: number; spans: readonly Span[] }
    | { kind: "code"; code: string; language?: string }
    | { kind: "rule" }
    | { kind: "neoui"; ui: UiDoc }
  );

interface Context {
  indent: number;
  quote: number;
  /** Inside a list item: blocks stack with a small gap. */
  tight: boolean;
}

function markerText(list: ListBlock, index: number): string {
  const checked = list.items[index]?.checked;
  if (checked !== undefined) return checked ? "✓" : "○";
  return list.ordered ? `${(list.start ?? 1) + index}.` : "•";
}

function flattenList(list: ListBlock, context: Context, out: MarkdownRow[]): void {
  const texts = list.items.map((_, index) => markerText(list, index));
  // The widest marker and a space: item text lines up whatever the marker.
  const width = Math.max(...texts.map((text) => text.length), 0) + 1;
  list.items.forEach((item, index) => {
    const start = out.length;
    const inner = { indent: context.indent + width, quote: context.quote, tight: true };
    flatten(item.blocks, inner, out);
    if (out.length === start) {
      out.push({ ...inner, kind: "paragraph", spans: [], markers: [], gap: "none" });
    }
    const first = out[start];
    if (first === undefined) return;
    first.markers.unshift({
      text: texts[index] ?? "",
      at: context.indent,
      width,
      done: item.checked === true,
    });
    if (index > 0) first.gap = "tight";
  });
}

/** A table as aligned monospace text, the way the chat showed tables before it had them. */
export function tableText(table: TableBlock): string {
  const rows = [table.header, ...table.rows].map((row) =>
    row.map((cell) => cell.spans.map((span) => span.text).join("")),
  );
  const columns = Math.max(...rows.map((row) => row.length));
  const widths = Array.from({ length: columns }, (_, c) =>
    Math.max(...rows.map((row) => (row[c] ?? "").length)),
  );
  const line = (row: string[]): string =>
    widths
      .map((width, c) => {
        const text = row[c] ?? "";
        return table.align?.[c] === "right" ? text.padStart(width) : text.padEnd(width);
      })
      .join(" │ ")
      .trimEnd();
  const [header = [], ...body] = rows;
  const rule = widths.map((width) => "─".repeat(width)).join("─┼─");
  return [line(header), rule, ...body.map(line)].join("\n");
}

function flatten(blocks: readonly Block[], context: Context, out: MarkdownRow[]): void {
  blocks.forEach((block, index) => {
    const start = out.length;
    const base = {
      indent: context.indent,
      quote: context.quote,
      markers: [],
      gap: "none" as RowGap,
    };
    switch (block.type) {
      case "paragraph":
        out.push({ ...base, kind: "paragraph", spans: block.spans });
        break;
      case "heading":
        out.push({ ...base, kind: "heading", level: block.level, spans: block.spans });
        break;
      case "code_block":
        out.push({ ...base, kind: "code", code: block.code, language: block.language });
        break;
      case "rule":
        out.push({ ...base, kind: "rule" });
        break;
      case "neoui":
        out.push({ ...base, kind: "neoui", ui: block.ui });
        break;
      case "table_block":
        out.push({ ...base, kind: "code", code: tableText(block) });
        break;
      case "quote":
        flatten(block.blocks, { ...context, quote: context.quote + 1 }, out);
        break;
      case "list":
        flattenList(block, context, out);
        break;
    }
    const first = out[start];
    if (first !== undefined && index > 0) first.gap = context.tight ? "tight" : "loose";
  });
}

/** The rows of each top-level block, the first block's and a later one's (they differ in gap). */
const rowsOfBlock = new WeakMap<Block, { first?: MarkdownRow[]; later?: MarkdownRow[] }>();

/** The rows of one top-level block: the same objects for as long as the block is the same. */
function blockRows(block: Block, first: boolean): MarkdownRow[] {
  let cached = rowsOfBlock.get(block);
  if (!cached) {
    cached = {};
    rowsOfBlock.set(block, cached);
  }
  const known = first ? cached.first : cached.later;
  if (known) return known;
  const rows: MarkdownRow[] = [];
  flatten([block], { indent: 0, quote: 0, tight: false }, rows);
  const [head] = rows;
  if (head && !first) head.gap = "loose";
  if (first) cached.first = rows;
  else cached.later = rows;
  return rows;
}

/**
 * The rows of a markdown text, top to bottom. A streaming message changes only its last blocks
 * and keeps the others, so their rows are kept too: the chat then redraws only what changed,
 * not the whole message for every chunk. `continued` blocks follow others of the same text, drawn
 * apart: the first of them keeps its gap.
 */
export function markdownRows(blocks: readonly Block[], continued = false): MarkdownRow[] {
  return blocks.flatMap((block, index) => blockRows(block, index === 0 && !continued));
}

/** What stands in front of a row's text, left to right: a gap of `chars` characters, or a marker. */
export interface IndentCell {
  chars: number;
  marker?: Marker;
}

export function indentCells(row: Pick<MarkdownRow, "indent" | "markers">): IndentCell[] {
  const cells: IndentCell[] = [];
  let cursor = 0;
  for (const marker of row.markers) {
    if (marker.at > cursor) cells.push({ chars: marker.at - cursor });
    cells.push({ chars: marker.width, marker });
    cursor = marker.at + marker.width;
  }
  if (row.indent > cursor) cells.push({ chars: row.indent - cursor });
  return cells;
}

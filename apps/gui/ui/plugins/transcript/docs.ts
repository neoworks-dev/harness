// The neoui docs the chat's rows draw that Ctrl-P can take the keyboard into: those with a field
// or a button. Plain functions, tested without a window.
import type { Block, UiDoc, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { isInteractive } from "./neoui/focus.ts";
import type { Row } from "./rows.ts";

/** A doc in the chat: the row that draws it, and what names its state. */
export interface DocTarget {
  row: number;
  ref: string;
  ui: UiDoc;
}

/** The docs among the blocks, those in lists and quotes too. */
function blockDocs(blocks: readonly Block[]): UiDoc[] {
  return blocks.flatMap((block): UiDoc[] => {
    if (block.type === "neoui") return [block.ui];
    if (block.type === "quote") return blockDocs(block.blocks);
    if (block.type === "list") return block.items.flatMap((item) => blockDocs(item.blocks));
    return [];
  });
}

function itemDocs(item: ViewItem): UiDoc[] {
  const component = item.component;
  if (component?.type === "neoui") return [component.ui];
  if (component?.type === "text") return blockDocs(component.blocks);
  return [];
}

function rowDocs(row: Row): { ref: string; docs: UiDoc[] } | undefined {
  if (row.kind === "prose") return { ref: row.ref, docs: blockDocs([row.block]) };
  // A split message's text is in the prose rows before it.
  if (row.kind === "message" && !row.split) return { ref: row.item.ref, docs: itemDocs(row.item) };
  if (row.kind === "item") return { ref: row.item.ref, docs: itemDocs(row.item) };
  return undefined;
}

/** The docs the rows draw that have a field or a button, top to bottom. */
export function interactiveDocs(rows: readonly Row[]): DocTarget[] {
  const out: DocTarget[] = [];
  rows.forEach((row, index) => {
    const found = rowDocs(row);
    if (found === undefined) return;
    for (const ui of found.docs) {
      if (isInteractive(ui.root)) out.push({ row: index, ref: found.ref, ui });
    }
  });
  return out;
}

export function sameDoc(a: DocTarget, b: DocTarget): boolean {
  return a.ref === b.ref && a.ui.id === b.ui.id;
}

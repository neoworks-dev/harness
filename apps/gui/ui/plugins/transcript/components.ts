// Small questions about view components that the chat's drawing code asks over and over. Plain
// TypeScript so they can be tested without a window.
import type { Block, Component, Space, Tone } from "@neoworks-dev/neo-protocol/types";
import type { Palette } from "../../lib/palette.ts";
import { isInteractive } from "./openui/focus.ts";

/** Whether the component is, or contains, a button: such an item is a question to answer. */
export function hasButtons(component: Component): boolean {
  switch (component.type) {
    case "button":
      return true;
    case "card":
    case "row":
    case "stack":
      return component.children.some(hasButtons);
    default:
      return false;
  }
}

/** Whether the component is an OpenUI doc with a field or a button to use. */
export function isForm(component: Component): boolean {
  return component.type === "openui" && isInteractive(component.ui.root);
}

/** The last part of a path (the whole of anything else). */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).at(-1) || trimmed;
}

/** The colour a tone stands for. */
export function toneColor(tone: Tone, p: Palette): string {
  return p[tone];
}

/** Pixels between children for a component's `gap`. */
export function gapOf(gap: Space | undefined, p: Palette): number {
  return p.space[gap ?? "s"];
}

/** Line height for text of `size`: a little air, as in Claude Code's chat. */
export function lineHeightOf(size: number): number {
  return Math.round(size * 1.5);
}

/** Collapses whitespace and line breaks, for the one-line forms. */
export function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** Markdown blocks as plain text, for copying and one-line previews: blocks apart by a blank line. */
export function blocksText(blocks: readonly Block[]): string {
  return blocks.map(blockText).join("\n\n");
}

function blockText(block: Block): string {
  switch (block.type) {
    case "paragraph":
    case "heading":
      return block.spans.map((span) => span.text).join("");
    case "code_block":
      return block.code.replace(/\n+$/, "");
    case "quote":
      return blocksText(block.blocks)
        .split("\n")
        .map((line) => `> ${line}`)
        .join("\n");
    case "list":
      return block.items
        .map((item, index) => {
          const marker = block.ordered ? `${(block.start ?? 1) + index}.` : "-";
          return `${marker} ${blocksText(item.blocks)}`;
        })
        .join("\n");
    case "rule":
      return "---";
    case "table_block":
      return [block.header, ...block.rows]
        .map((row) => row.map((cell) => cell.spans.map((span) => span.text).join("")).join(" | "))
        .join("\n");
    case "openui":
      // A drawn UI has no text of its own to copy.
      return "";
  }
}

/** What a component says as plain text: its texts, code and key-value lines, one per line. */
export function componentText(component: Component): string {
  switch (component.type) {
    case "text":
      return blocksText(component.blocks);
    case "code":
      return component.code.replace(/\n+$/, "");
    case "card":
    case "row":
    case "stack":
      return component.children
        .map(componentText)
        .filter((text) => text !== "")
        .join("\n");
    case "kv":
      return component.items.map((item) => `${item.key}: ${item.value}`).join("\n");
    default:
      return "";
  }
}

/**
 * The lines of a message as plain text, for measuring how wide it wants to be: the text of each
 * paragraph and heading, split at line breaks. Undefined for anything but plain paragraphs and
 * headings (lists, code, quotes), which take the full width they are given.
 */
export function plainLines(component: Component): string[] | undefined {
  if (component.type !== "text") return undefined;
  const lines: string[] = [];
  for (const block of component.blocks) {
    if (block.type !== "paragraph" && block.type !== "heading") return undefined;
    lines.push(
      ...block.spans
        .map((span) => span.text)
        .join("")
        .split("\n"),
    );
  }
  return lines;
}

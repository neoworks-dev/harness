// Small questions about view components that the chat's drawing code asks over and over. Plain
// TypeScript so they can be tested without a window.
import type { Component, Space, Tone } from "@suite/protocol/types";
import type { Palette } from "../../lib/palette.ts";

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

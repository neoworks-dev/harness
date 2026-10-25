// happy-dom plumbing for the tests: a fresh document per test, and a deterministic, readable
// serialization for the snapshot files.
import { Window } from "happy-dom";

export interface Page {
  window: Window;
  container: HTMLElement;
}

export function newPage(): Page {
  const window = new Window({ url: "http://localhost/" });
  const container = window.document.createElement("div");
  window.document.body.append(container);
  // happy-dom's classes are structurally the DOM's; its typings just don't say so.
  return { window, container: container as unknown as HTMLElement };
}

const INLINE = new Set([
  "a",
  "b",
  "code",
  "del",
  "em",
  "i",
  "img",
  "input",
  "kbd",
  "s",
  "small",
  "span",
  "strong",
]);

/** Diff lines are spans that display as blocks. */
function isInline(node: Element): boolean {
  return INLINE.has(node.tagName.toLowerCase()) && !node.classList.contains("neo-diff-line");
}

function isInlineOnly(node: Element): boolean {
  return Array.from(node.children).every((child) => isInline(child) && isInlineOnly(child));
}

/** One element per line, indented; elements with only inline content stay on one line. */
function pretty(node: Element, depth: number, out: string[]): void {
  const pad = "  ".repeat(depth);
  if (isInlineOnly(node)) {
    out.push(pad + node.outerHTML);
    return;
  }
  const open = node.outerHTML.slice(0, node.outerHTML.indexOf(">") + 1);
  out.push(pad + open);
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === 1) pretty(child as unknown as Element, depth + 1, out);
    else if (child.textContent?.trim()) out.push(`${pad}  ${child.textContent}`);
  }
  out.push(`${pad}</${node.tagName.toLowerCase()}>`);
}

export function prettyHtml(container: Element): string {
  const out: string[] = [];
  for (const child of Array.from(container.children)) pretty(child, 0, out);
  return `${out.join("\n")}\n`;
}

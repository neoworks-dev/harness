// An issue's or comment's markdown as the chat's blocks, parsed by the same parser the daemon
// gives replies (daemon/src/view/markdown.ts), so the GitHub view draws it as the chat does.
import type { Block } from "@neoworks-dev/neo-protocol/types";
import { parseMarkdown } from "../../../../../daemon/src/view/markdown.ts";
import { readable } from "./github.ts";

const parsed = new Map<string, Block[]>();

/** The blocks of `markdown`, without the HTML comments issue templates are full of; kept per text. */
export function blocksOf(markdown: string): Block[] {
  const text = readable(markdown);
  let blocks = parsed.get(text);
  if (blocks === undefined) {
    blocks = text === "" ? [] : parseMarkdown(text);
    if (parsed.size > 200) parsed.clear();
    parsed.set(text, blocks);
  }
  return blocks;
}

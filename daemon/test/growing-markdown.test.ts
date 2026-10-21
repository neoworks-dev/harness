import { expect, test } from "bun:test";
import type { UiDoc } from "@suite/protocol/types";
import { GrowingMarkdown, parseMarkdown } from "../src/view/markdown.ts";

const MESSAGE = [
  "# Title\n\nA paragraph with **bold**, `code` and a [link](https://x.dev).\nIt goes on",
  " over two lines.\n\nSetext heading\n---\n\n- one\n- two\n  continued\n\n  still two\n- three\n",
  "\n1. first\n2. second\n\n> a quote\nlazy line\n\n```ts\nconst a = 1;\n\n",
  "const b = 2;\n```\n\n| a | b |\n| - | -: |\n| 1 | 2 |\n| 3 | 4 |\n\n***\n\n",
  "```neoui\n<Stack><Text>hi</Text></Stack>\n```\n\nAfter the program.\n\n",
  "```neoui\n<Text>open</Text>\n",
].join("");

const docs = (index: number, program: string, closed: boolean): UiDoc => ({
  id: `${index}:${closed}:${program.length}`,
});

test("a message parsed as it grows says what parsing it whole says, at every length", () => {
  const growing = new GrowingMarkdown();
  for (let end = 0; end <= MESSAGE.length; end++) {
    const source = MESSAGE.slice(0, end);
    expect(growing.parse(source, docs)).toEqual(parseMarkdown(source, docs));
  }
});

test("chunks of any size give the same blocks, and settled blocks stay the same objects", () => {
  for (const size of [3, 17, 64]) {
    const growing = new GrowingMarkdown();
    let previous: unknown[] = [];
    let kept = 0;
    for (let end = size; end < MESSAGE.length + size; end += size) {
      const source = MESSAGE.slice(0, end);
      const blocks = growing.parse(source, docs);
      expect(blocks).toEqual(parseMarkdown(source, docs));
      if (previous.length > 1 && blocks[0] === previous[0]) kept++;
      previous = blocks;
    }
    expect(kept).toBeGreaterThan(0);
  }
});

test("a link definition or a carriage return means parsing it all", () => {
  const growing = new GrowingMarkdown();
  const text = "See [the docs][d].\n\nMore text.\n\n[d]: https://x.dev\n";
  for (let end = 0; end <= text.length; end++) {
    const source = text.slice(0, end);
    expect(growing.parse(source)).toEqual(parseMarkdown(source));
  }
  const crlf = "one\r\n\r\ntwo\r\n";
  expect(new GrowingMarkdown().parse(crlf)).toEqual(parseMarkdown(crlf));
});

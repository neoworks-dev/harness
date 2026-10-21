import { expect, test } from "bun:test";
import type { Block } from "@neoworks-dev/neo-protocol/types";
import { paletteOf } from "../ui/lib/palette.ts";
import { baseName, lineHeightOf } from "../ui/plugins/transcript/components.ts";
import { fitImage } from "../ui/plugins/transcript/image.ts";
import { isLink, linkHint, spanStyle } from "../ui/plugins/transcript/runs.ts";
import { markdownRows } from "../ui/plugins/transcript/markdown.ts";
import { columnWeights } from "../ui/plugins/transcript/table.ts";

const p = paletteOf();

test("inline code is mono on a raised chip, links are accent and underlined", () => {
  expect(spanStyle({ text: "x", code: true }, p)).toMatchObject({
    fontFamily: "mono",
    background: p.raised,
  });
  expect(spanStyle({ text: "x", bold: true, italic: true }, p)).toEqual({
    fontWeight: "bold",
    italic: true,
  });
  expect(spanStyle({ text: "x", href: "https://a.b" }, p)).toMatchObject({
    color: p.accent,
    underline: true,
  });
  expect(isLink({ text: "x" })).toBe(false);
});

test("a link shows its address unless the text is the address", () => {
  expect(linkHint({ text: "docs", href: "https://a.b" })).toBe("https://a.b");
  expect(linkHint({ text: "https://a.b", href: "https://a.b" })).toBeUndefined();
});

test("a picture is scaled down to fit, never up", () => {
  expect(fitImage({ width: 1000, height: 500 }, { width: 400, height: 400 })).toEqual({
    width: 400,
    height: 200,
  });
  expect(fitImage({ width: 100, height: 50 }, { width: 400, height: 400 })).toEqual({
    width: 100,
    height: 50,
  });
});

test("table columns share the width by their longest cell, within bounds", () => {
  const weights = columnWeights({
    type: "table",
    columns: ["id", "description"],
    rows: [["1", "a".repeat(100)]],
  });
  expect(weights).toEqual([4, 40]);
  expect(columnWeights({ type: "table", columns: [], rows: [] })).toEqual([]);
});

test("names and line heights", () => {
  expect(baseName("/a/b/c.txt")).toBe("c.txt");
  expect(baseName("dir/")).toBe("dir");
  expect(lineHeightOf(14)).toBe(21);
});

test("nested markdown flattens to rows with an indent, markers and quote depth", () => {
  const para = (text: string): Block => ({ type: "paragraph", spans: [{ text }] });
  const rows = markdownRows([
    para("intro"),
    {
      type: "list",
      ordered: true,
      items: [
        { blocks: [para("one")] },
        {
          blocks: [
            para("two"),
            { type: "list", ordered: false, items: [{ blocks: [para("nested")] }] },
          ],
        },
      ],
    },
    { type: "quote", blocks: [para("quoted")] },
  ]);
  const shape = rows.map((row) => ({
    text: row.kind === "paragraph" ? row.spans[0]?.text : row.kind,
    indent: row.indent,
    markers: row.markers.map((marker) => `${marker.text}@${marker.at}`),
    quote: row.quote,
    gap: row.gap,
  }));
  expect(shape).toEqual([
    { text: "intro", indent: 0, markers: [], quote: 0, gap: "none" },
    { text: "one", indent: 3, markers: ["1.@0"], quote: 0, gap: "loose" },
    { text: "two", indent: 3, markers: ["2.@0"], quote: 0, gap: "tight" },
    { text: "nested", indent: 5, markers: ["•@3"], quote: 0, gap: "tight" },
    { text: "quoted", indent: 0, markers: [], quote: 1, gap: "loose" },
  ]);
});

test("a list item that starts with a list keeps both markers on one row", () => {
  const para = (text: string): Block => ({ type: "paragraph", spans: [{ text }] });
  const rows = markdownRows([
    {
      type: "list",
      ordered: false,
      items: [{ blocks: [{ type: "list", ordered: false, items: [{ blocks: [para("x")] }] }] }],
    },
  ]);
  expect(rows).toHaveLength(1);
  expect(rows[0]?.markers.map((marker) => marker.at)).toEqual([0, 2]);
});

test("markdown rows of blocks that did not change are the same objects, as a message streams", () => {
  const para = (text: string): Block => ({ type: "paragraph", spans: [{ text }] });
  const first = para("one");
  const list: Block = {
    type: "list",
    ordered: false,
    items: [{ blocks: [para("a")] }, { blocks: [para("b")] }],
  };
  const before = markdownRows([first, list, para("thr")]);
  const after = markdownRows([first, list, para("three")]);
  expect(after.slice(0, 3)).toEqual(before.slice(0, 3));
  for (let i = 0; i < 3; i++) expect(after[i]).toBe(before[i]);
  expect(after[3]).not.toBe(before[3]);
  // A block's first row has a gap only when the block isn't the first.
  expect(markdownRows([list])[0]?.gap).toBe("none");
  expect(markdownRows([first, list])[1]?.gap).toBe("loose");
});

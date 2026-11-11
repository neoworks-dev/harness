import { expect, test } from "bun:test";
import type { Block, UiDoc, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { interactiveDocs } from "../ui/plugins/transcript/docs.ts";
import type { Row } from "../ui/plugins/transcript/rows.ts";

const doc = (id: string, interactive: boolean): UiDoc => ({
  id,
  root: {
    component: "Card",
    props: {
      children: [
        interactive
          ? { component: "Button", key: "go", props: { label: "Go" } }
          : { component: "Text", props: { text: "hi" } },
      ],
    },
  },
});

const neoui = (ui: UiDoc): Block => ({ type: "neoui", ui });
const prose = (ref: string, block: Block): Row => ({ kind: "prose", key: ref, ref, block, first: true });
const message = (ref: string, blocks: Block[], split = false): Row => {
  const item: ViewItem = { ref, kind: "agent", component: { type: "text", blocks } };
  return { kind: "message", key: ref, item, final: false, changes: [], split };
};

test("only docs with a field or a button are found, top to bottom, with their row", () => {
  const rows: Row[] = [
    prose("a", neoui(doc("1", true))),
    prose("a", neoui(doc("2", false))),
    message("b", [neoui(doc("3", true))]),
  ];
  expect(interactiveDocs(rows).map((d) => [d.row, d.ref, d.ui.id])).toEqual([
    [0, "a", "1"],
    [2, "b", "3"],
  ]);
});

test("a split message's docs are its prose rows', not found twice; docs in a list are found", () => {
  const listed: Block = {
    type: "list",
    ordered: false,
    items: [{ blocks: [neoui(doc("4", true))] }],
  } as Block;
  const rows: Row[] = [prose("c", neoui(doc("5", true))), message("c", [neoui(doc("5", true))], true), prose("d", listed)];
  expect(interactiveDocs(rows).map((d) => d.ui.id)).toEqual(["5", "4"]);
});

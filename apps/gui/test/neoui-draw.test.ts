import { expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Block, Component, UiDoc, UiNode, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { COMPONENTS } from "@neoworks/neoui";
import { isForm } from "../ui/plugins/transcript/components.ts";
import { blocksText } from "../ui/plugins/transcript/components.ts";
import { focusOrder, isInteractive } from "../ui/plugins/transcript/neoui/focus.ts";
import { markdownRows } from "../ui/plugins/transcript/markdown.ts";

const here = import.meta.dir;

/** What a node under another is for, not a component of its own on the page. */
const PARTS = new Set([
  "Tab",
  "AccordionItem",
  "Step",
  "KV",
  "ListItem",
  "Series",
  "Slice",
  "Option",
  "OptionCard",
  "CheckBoxItem",
  "SwitchItem",
  "FollowUpItem",
]);

/** The names of the library a window draws: neoui's control elements (Query, For, Row…) are not. */
function drawable(): string[] {
  const control = new Set(["Query", "State", "For", "If", "Row"]);
  return COMPONENTS.map((c) => c.name).filter((name) => !control.has(name));
}

test("every component of neo@1 library is drawn", () => {
  const source = readFileSync(join(here, "../ui/plugins/transcript/neoui/Node.svelte"), "utf8");
  const drawn = [...source.matchAll(/^\s{4}(\w+): /gm)].map((match) => match[1]);
  const missing = drawable().filter((name) => !PARTS.has(name) && !drawn.includes(name));
  expect(missing).toEqual([]);
});

const doc = (root: UiNode, streaming = false): UiDoc => ({ id: "0", root, streaming });
const block = (ui: UiDoc): Block => ({ type: "neoui", ui });

test("a neoui block among the markdown is a row of its own, in its place", () => {
  const ui = doc({ component: "Card", props: { children: [] } });
  const rows = markdownRows([
    { type: "paragraph", spans: [{ text: "before" }] },
    block(ui),
    { type: "paragraph", spans: [{ text: "after" }] },
  ]);
  expect(rows.map((row) => row.kind)).toEqual(["paragraph", "neoui", "paragraph"]);
  const drawn = rows[1];
  expect(drawn?.kind === "neoui" && drawn.ui).toBe(ui);
  expect(drawn?.gap).toBe("loose");
});

test("a drawn UI has no text of its own to copy", () => {
  const ui = doc({ component: "Text", props: {} });
  expect(blocksText([{ type: "paragraph", spans: [{ text: "a" }] }, block(ui)])).toBe("a\n\n");
});

test("a tool result that is a UI with a field or a button is open for use, one that only shows is not", () => {
  const form: Component = {
    type: "neoui",
    ui: doc({ component: "Button", key: "go", props: { label: "Go" } }),
  };
  const page: Component = {
    type: "neoui",
    ui: doc({ component: "Card", props: { children: [{ component: "Text", props: {} }] } }),
  };
  expect(isForm(form)).toBe(true);
  expect(isForm(page)).toBe(false);
  expect(isForm({ type: "text", blocks: [] })).toBe(false);
});

/** Every doc of the fixtures, whatever carries it. */
function docsOf(item: ViewItem): UiDoc[] {
  const out: UiDoc[] = [];
  const visit = (component: Component): void => {
    if (component.type === "neoui") out.push(component.ui);
    else if (component.type === "text") {
      for (const part of component.blocks) if (part.type === "neoui") out.push(part.ui);
    } else if ("children" in component) component.children.forEach(visit);
  };
  visit(item.component);
  return out;
}

test("the fixtures' docs only use components the window draws and have their targets in order", () => {
  const dir = join(here, "../../../fixtures");
  const known = new Set(COMPONENTS.map((c) => c.name));
  let seen = 0;
  let targets = 0;
  for (const name of readdirSync(dir).filter((entry) => entry.startsWith("neoui-"))) {
    const items: ViewItem[] = JSON.parse(readFileSync(join(dir, name, "resolved.json"), "utf8"));
    for (const ui of items.flatMap(docsOf)) {
      seen += 1;
      const walk = (node: UiNode): void => {
        expect(known.has(node.component)).toBe(true);
        for (const value of Object.values(node.props)) {
          const list = Array.isArray(value) ? value.flat() : [value];
          for (const entry of list) {
            if (typeof entry === "object" && entry !== null && "component" in entry)
              walk(entry as UiNode);
          }
        }
      };
      if (ui.root !== undefined) {
        walk(ui.root);
        const order = focusOrder(ui.root, { tab: () => 0, isOpen: (_, starts) => starts });
        expect(new Set(order).size).toBe(order.length);
        targets += order.length;
        if (order.length > 0)
          expect(isInteractive(ui.root) || order.some((id) => id.startsWith("/"))).toBe(true);
      }
    }
  }
  expect(seen).toBeGreaterThanOrEqual(5);
  expect(targets).toBeGreaterThan(0);
});

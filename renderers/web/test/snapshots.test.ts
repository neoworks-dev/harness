import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Theme, ViewItem } from "@suite/protocol/types";
import { mountChat } from "../src/render.ts";
import { newPage, prettyHtml } from "./dom.ts";

const FIXTURES = join(import.meta.dir, "../../../fixtures");
const UPDATE = process.env.UPDATE_SNAPSHOTS === "1";

const names = readdirSync(FIXTURES, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && existsSync(join(FIXTURES, entry.name, "resolved.json")))
  .map((entry) => entry.name)
  .sort();

async function load<T>(path: string): Promise<T> {
  return Bun.file(path).json();
}

describe("fixtures", () => {
  test("there are fixtures to draw", () => {
    expect(names.length).toBeGreaterThan(0);
  });

  for (const name of names) {
    test(name, async () => {
      const theme = await load<Theme>(
        join(import.meta.dir, "../../../protocol/themes/default.json"),
      );
      const items = await load<ViewItem[]>(join(FIXTURES, name, "resolved.json"));
      const { container } = newPage();
      mountChat(container, { theme }).setItems(items);

      const html = prettyHtml(container);
      const file = join(FIXTURES, name, "web.html");
      if (UPDATE) await Bun.write(file, html);
      else expect(html).toBe(await Bun.file(file).text());
    });
  }
});

import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { ViewItem } from "@suite/protocol/types";
import { COMPONENTS } from "@neoworks/neoui";
import { validateDef } from "@suite/protocol/validate";
import { resolveFixture, type Fixture } from "../src/view/fixtures.ts";

const dir = join(import.meta.dir, "..", "..", "fixtures");
const names = readdirSync(dir)
  .filter((name) => existsSync(join(dir, name, "fixture.json")))
  .sort();

function read<T>(name: string, file: string): T {
  return JSON.parse(readFileSync(join(dir, name, file), "utf8"));
}

describe("fixtures", () => {
  for (const name of names) {
    test(`${name}: resolved.json is what the daemon resolves, and valid`, async () => {
      const items = await resolveFixture(name, read<Fixture>(name, "fixture.json"));
      expect(items).toEqual(read<ViewItem[]>(name, "resolved.json"));
      for (const item of items) {
        expect(validateDef("view.schema.json#/$defs/ViewItem", item)).toBeUndefined();
      }
    });
  }

  test("the neoui fixtures are there: a template, a chat, a model's dashboard, a form with inputs and a press, a grid, boxes and shapes, the rest of the library, a stream", () => {
    expect(names.filter((n) => n.startsWith("openui-"))).toEqual([
      "openui-boxes",
      "openui-chat",
      "openui-dashboard",
      "openui-disk",
      "openui-form",
      "openui-grid",
      "openui-library",
      "openui-streaming",
    ]);
  });

  // Renderers draw the fixtures and compare them with snapshots, so a component that no fixture
  // uses is one a renderer could leave out without a test failing.
  test("the neoui fixtures use every component of the library", () => {
    const used = new Set<string>();
    const visit = (value: unknown): void => {
      if (Array.isArray(value)) value.forEach(visit);
      else if (typeof value === "object" && value !== null) {
        if ("component" in value && typeof value.component === "string") used.add(value.component);
        Object.values(value).forEach(visit);
      }
    };
    for (const name of names.filter((n) => n.startsWith("openui-"))) {
      visit(read(name, "resolved.json"));
    }
    // The control elements are never drawn.
    const control = new Set(["Query", "State", "For", "If", "Row"]);
    const drawn = COMPONENTS.map((c) => c.name).filter((name) => !control.has(name));
    expect(drawn.filter((name) => !used.has(name))).toEqual([]);
  });
});

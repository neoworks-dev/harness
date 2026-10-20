import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { ViewItem } from "@suite/protocol/types";
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

  test("the OpenUI fixtures are there: a template, a chat, a model's dashboard, a form with inputs and a press, a grid, a stream", () => {
    expect(names.filter((n) => n.startsWith("openui-"))).toEqual([
      "openui-chat",
      "openui-dashboard",
      "openui-disk",
      "openui-form",
      "openui-grid",
      "openui-streaming",
    ]);
  });
});

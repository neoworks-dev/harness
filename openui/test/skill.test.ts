import { expect, test } from "bun:test";
import type { AppComponent } from "@suite/protocol/types";
import { appLibrary, type AppComponents } from "../src/components.ts";
import { library } from "../src/library.ts";
import { appComponentsText, skillText, uiLibraryText } from "../src/skill.ts";

const BRIGHTNESS: AppComponent = {
  description: "An image with a brightness slider and live preview. ",
  params: { asset: "string", name: "string?" },
  openui: "root = Card([])",
};

const apps: AppComponents = { "photo-editor": { BrightnessCard: BRIGHTNESS } };

test("the text lists every component of the library with its arguments in order, as the library is now", () => {
  const text = skillText();
  // Each signature comes from the library, so a component added to it is here without a regeneration.
  for (const [name, signature] of Object.entries(library.toSpec().components)) {
    expect(text).toContain(`${name}(`);
    expect(text).toContain(signature.signature);
  }
  expect(text).toContain(
    'Button(label: string, action?: ActionExpression, variant?: "primary" | "secondary" | "danger")',
  );
  expect(text).toContain("- @Copy(text: string)");
  // It is for a tool result, not a skill file.
  expect(text.startsWith("---")).toBe(false);
  expect(text.startsWith("# OpenUI")).toBe(true);
});

test("it says when to use UI, how to lay it out and what a Box and a Shape are for", () => {
  const text = skillText();
  expect(text).toContain("## When to use it");
  expect(text).toContain("## Layout");
  expect(text).toContain("Use `Box` and `Shape` only for what no component above is");
  expect(text).toContain("## App components");
});

test("the components of connected apps follow, each with its signature and what it is for", () => {
  const lib = appLibrary(apps);
  const open = new Set(["photo-editor"]);
  expect(appComponentsText(lib, open)).toBe(
    "## photo-editor\nBrightnessCard(asset: string, name?: string) — An image with a brightness slider and live preview.",
  );
  const text = uiLibraryText(lib, open);
  expect(text.startsWith(skillText())).toBe(true);
  expect(text.endsWith("preview.\n")).toBe(true);
  expect(text).toContain("# App components");
  expect(uiLibraryText(lib, new Set())).toContain("No connected app adds components right now.");
});

test("a component whose plain name is taken is listed by its qualified name", () => {
  const lib = appLibrary({ ...apps, notes: { BrightnessCard: BRIGHTNESS } });
  expect(appComponentsText(lib, new Set(["notes"]))).toStartWith(
    "## notes\nNotesBrightnessCard(asset: string, name?: string)",
  );
});

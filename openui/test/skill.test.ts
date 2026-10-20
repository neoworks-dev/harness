import { expect, test } from "bun:test";
import { type AppComponents } from "../src/components.ts";
import { library } from "../src/library.ts";
import { appComponentsText, skillText, uiLibraryText } from "../src/skill.ts";

const apps: AppComponents = {
  "photo-editor": {
    BrightnessCard: {
      description: "An image with a brightness slider and live preview. ",
      params: { asset: "string", name: "string?" },
      openui: "root = Card([])",
    },
  },
};

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
  expect(appComponentsText(apps)).toBe(
    "## photo-editor\nBrightnessCard(asset: string, name?: string) — An image with a brightness slider and live preview.",
  );
  const text = uiLibraryText(apps);
  expect(text.startsWith(skillText())).toBe(true);
  expect(text.endsWith("preview.\n")).toBe(true);
  expect(text).toContain("# App components");
  expect(uiLibraryText({})).toContain("No connected app adds components right now.");
});

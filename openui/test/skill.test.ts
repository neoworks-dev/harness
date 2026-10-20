import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { SKILL_PATH, skillMarkdown } from "../src/skill.ts";

test("harness/skills/openui/SKILL.md is what the library produces (bun run --cwd openui skill)", () => {
  const onDisk = readFileSync(resolve(import.meta.dir, "..", "..", SKILL_PATH), "utf8");
  expect(onDisk).toBe(skillMarkdown());
});

test("the skill says when to load it and lists every component with its arguments in order", () => {
  const text = skillMarkdown();
  expect(text.startsWith("---\nname: openui\ndescription: ")).toBe(true);
  expect(text).toContain("Load it whenever the user asks to show");
  expect(text).toContain(
    'Button(label: string, action?: ActionExpression, variant?: "primary" | "secondary" | "danger")',
  );
  expect(text).toContain("- @Copy(text: string)");
});

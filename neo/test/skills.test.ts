import { expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { loadSkills, parseSkill, type SkillDirs } from "../src/skills.ts";
import { SYSTEM_PROMPT, systemPrompt } from "../src/system-prompt.ts";
import { createCoreTools } from "../src/tools/core.ts";
import { createSkillTool } from "../src/tools/skill.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

async function skill(dir: string, folder: string, text: string): Promise<void> {
  await mkdir(join(dir, folder), { recursive: true });
  await writeFile(join(dir, folder, "SKILL.md"), text);
}

const doc = (name: string, description: string, body: string): string =>
  `---\nname: ${name}\ndescription: ${description}\n---\n\n${body}\n`;

test("parseSkill reads the frontmatter, quoted or not, and leaves the body", () => {
  expect(parseSkill(doc("a", "Does a thing", "# A\n\ntext"), "dir")).toEqual({
    name: "a",
    description: "Does a thing",
    body: "# A\n\ntext",
  });
  expect(parseSkill("---\nname: \"q\"\ndescription: 'when: needed'\n---\nbody", "dir")).toEqual({
    name: "q",
    description: "when: needed",
    body: "body",
  });
});

test("parseSkill names a skill after its directory, and skips one with no description or frontmatter", () => {
  expect(parseSkill("---\ndescription: d\n---\nb", "folder")?.name).toBe("folder");
  expect(parseSkill("---\nname: x\n---\nb", "x")).toBeUndefined();
  expect(parseSkill("just text", "x")).toBeUndefined();
});

test("a long description is cut", () => {
  const long = "word ".repeat(200);
  const description = parseSkill(doc("a", long, "b"), "a")?.description ?? "";
  expect(description.length).toBeLessThanOrEqual(400);
  expect(description.endsWith("…")).toBe(true);
});

test("skills come from the bundled, the user's and the project's directories; later ones win by name", async () => {
  const root = await tempDir();
  const dirs: SkillDirs = { bundled: join(root, "bundled"), user: join(root, "user") };
  const cwd = join(root, "project");
  await skill(dirs.bundled, "neoui", doc("neoui", "bundled neoui", "one"));
  await skill(dirs.bundled, "other", doc("other", "bundled other", "two"));
  await skill(dirs.user, "neoui", doc("neoui", "user neoui", "three"));
  await skill(join(cwd, ".neo", "skills"), "mine", doc("mine", "project only", "four"));
  await skill(join(cwd, ".neo", "skills"), "other", doc("other", "project other", "five"));
  await skill(dirs.user, "broken", "no frontmatter");

  const skills = loadSkills(cwd, dirs);
  expect(skills.map((s) => [s.name, s.description, s.body])).toEqual([
    ["neoui", "user neoui", "three"],
    ["other", "project other", "five"],
    ["mine", "project only", "four"],
  ]);
  expect(skills[0]?.path).toBe(join(dirs.user, "neoui", "SKILL.md"));
});

test("missing directories are no skills, and without directories there are none", async () => {
  const root = await tempDir();
  expect(loadSkills(root, { bundled: join(root, "a"), user: join(root, "b") })).toEqual([]);
  expect(loadSkills(root, undefined)).toEqual([]);
});

test("the system prompt lists each skill's name and description and says how to load one", () => {
  const prompt = systemPrompt([
    { name: "neoui", description: "Show UI", body: "", path: "" },
    { name: "x", description: "Other", body: "", path: "" },
  ]);
  expect(prompt.startsWith(SYSTEM_PROMPT)).toBe(true);
  expect(prompt).toContain("skill {name}");
  expect(prompt).toContain("- neoui: Show UI\n- x: Other");
  expect(systemPrompt([])).toBe(SYSTEM_PROMPT);
});

test("the skill tool returns the body; an unknown name is an error that lists the names", async () => {
  const tool = createSkillTool([
    { name: "neoui", description: "d", body: "line 1\nline 2", path: "" },
    { name: "other", description: "d", body: "x", path: "" },
  ]);
  const result = await tool.execute("t", { name: "neoui" });
  expect(resultText(result)).toBe("line 1\nline 2");
  expect(result.details).toEqual({ name: "neoui", lines: 2 });
  expect(resultText(await tool.execute("t", { name: " NeoUI " }))).toBe("line 1\nline 2");
  expect(await errorMessage(tool.execute("t", { name: "nope" }))).toBe(
    'No skill named "nope". Skills: neoui, other.',
  );
  expect(await errorMessage(createSkillTool([]).execute("t", { name: "x" }))).toContain(
    "There are no skills.",
  );
});

test("skill is one of the core tools", () => {
  const names = createCoreTools("/tmp", undefined).map((tool) => tool.name);
  expect(names).toContain("skill");
});

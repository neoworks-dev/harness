// Writes harness/skills/openui/SKILL.md from the suite@1 library.
//   bun run --cwd openui skill
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { SKILL_PATH, skillMarkdown } from "../src/skill.ts";

const out = resolve(import.meta.dir, "..", "..", SKILL_PATH);
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, skillMarkdown());
process.stdout.write(`wrote ${out}\n`);

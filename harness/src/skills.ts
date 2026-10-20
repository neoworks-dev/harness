// Skills: instructions neo loads when the work calls for them, kept out of the system prompt
// until then. A skill is a directory with a SKILL.md: frontmatter `name` and `description`, then
// the text. They come from the harness itself (harness/skills), the user's
// ~/.config/suite/skills and the session's <cwd>/.suite/skills; a later one replaces an earlier
// one of the same name.
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

export interface Skill {
  name: string;
  /** When to load it: what the system prompt lists. */
  description: string;
  /** The text the `skill` tool returns. */
  body: string;
  /** The SKILL.md it came from. */
  path: string;
}

/** Where skills are looked for. The session's own is `<cwd>/.suite/skills`. */
export interface SkillDirs {
  bundled: string;
  user: string;
}

/** A description longer than this is cut: the prompt carries one for every skill. */
const DESCRIPTION_LIMIT = 400;

function unquote(value: string): string {
  const quote = value[0];
  if ((quote === '"' || quote === "'") && value.length >= 2 && value.endsWith(quote)) {
    return value.slice(1, -1);
  }
  return value;
}

/**
 * A SKILL.md: `---` lines around `key: value` lines, then the body. Values are one line, quoted or
 * not. Undefined without frontmatter. The name defaults to `fallbackName` (the directory's).
 */
export function parseSkill(text: string, fallbackName: string): Omit<Skill, "path"> | undefined {
  const match = /^---[ \t]*\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  if (!match?.[1]) return undefined;
  const fields = new Map<string, string>();
  for (const line of match[1].split(/\r?\n/)) {
    const colon = line.indexOf(":");
    if (colon <= 0 || /^\s/.test(line)) continue;
    fields.set(line.slice(0, colon).trim(), unquote(line.slice(colon + 1).trim()));
  }
  const name = fields.get("name") || fallbackName;
  const description = (fields.get("description") ?? "").replace(/\s+/g, " ").trim();
  if (description === "") return undefined;
  const cut =
    description.length > DESCRIPTION_LIMIT
      ? `${description.slice(0, DESCRIPTION_LIMIT - 1)}…`
      : description;
  return { name, description: cut, body: text.slice(match[0].length).trim() };
}

/** The skills under one directory: `<dir>/<skill>/SKILL.md`. A missing directory has none. */
function readSkills(dir: string): Skill[] {
  let entries: string[];
  try {
    entries = readdirSync(dir).sort();
  } catch {
    return [];
  }
  const skills: Skill[] = [];
  for (const entry of entries) {
    const path = join(dir, entry, "SKILL.md");
    let text: string;
    try {
      text = readFileSync(path, "utf8");
    } catch {
      continue;
    }
    const parsed = parseSkill(text, basename(entry));
    if (parsed) skills.push({ ...parsed, path });
  }
  return skills;
}

/** The skills a session has: bundled, then the user's, then the project's, by name. */
export function loadSkills(cwd: string, dirs: SkillDirs | undefined): Skill[] {
  if (!dirs) return [];
  const byName = new Map<string, Skill>();
  for (const dir of [dirs.bundled, dirs.user, join(cwd, ".suite", "skills")]) {
    for (const skill of readSkills(dir)) byName.set(skill.name, skill);
  }
  return [...byName.values()];
}

// Skills: instructions neo loads when the work calls for them, kept out of the system prompt
// until then. A skill is a directory with a SKILL.md: frontmatter `name` and `description`, then
// the text. They come from the harness itself (neo/skills), the user's
// ~/.config/suite/skills and the session's <cwd>/.suite/skills; a later one replaces an earlier
// one of the same name.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { declaredPaths } from "./plugin-parts.ts";
import { enabledPlugins, expandPluginRoot, type Plugin } from "./plugins.ts";

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
  /** The plugins directory (plugins.ts): the skills of the enabled plugins, under the others. */
  plugins?: string;
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
function readSkills(dir: string, only?: string): Skill[] {
  let entries: string[];
  try {
    entries = readdirSync(dir).sort();
  } catch {
    return [];
  }
  if (only !== undefined) entries = entries.filter((entry) => entry === only);
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

/** The skills of one enabled plugin: `skills/`, and the paths its manifest or marketplace entry names. */
export function readPluginSkills(plugin: Plugin): Skill[] {
  const skills: Skill[] = [];
  for (const dir of [join(plugin.root, "skills"), ...declaredPaths(plugin, "skills")]) {
    // A path is a directory of skills, or one skill's own directory.
    const found = existsSync(join(dir, "SKILL.md"))
      ? readSkills(dirname(dir), basename(dir))
      : readSkills(dir);
    for (const skill of found) {
      const note = `Base directory for this skill: ${dirname(skill.path)}`;
      const body = `${expandPluginRoot(skill.body, plugin)}\n\n${note}`;
      if (!skills.some((other) => other.path === skill.path)) skills.push({ ...skill, body });
    }
  }
  return skills;
}

/**
 * The enabled plugins' skills. A name two plugins both use is written `plugin:name` for each, so
 * neither hides the other.
 */
function pluginSkills(dir: string): Skill[] {
  const all = enabledPlugins(dir).flatMap((plugin) =>
    readPluginSkills(plugin).map((skill) => ({ plugin, skill })),
  );
  const counts = new Map<string, number>();
  for (const { skill } of all) counts.set(skill.name, (counts.get(skill.name) ?? 0) + 1);
  return all.map(({ plugin, skill }) =>
    (counts.get(skill.name) ?? 0) > 1 ? { ...skill, name: `${plugin.name}:${skill.name}` } : skill,
  );
}

/**
 * The skills a session has: the enabled plugins', then bundled, the user's and the project's, by
 * name (a later one replaces an earlier one).
 */
export function loadSkills(cwd: string, dirs: SkillDirs | undefined): Skill[] {
  if (!dirs) return [];
  const byName = new Map<string, Skill>();
  if (dirs.plugins) for (const skill of pluginSkills(dirs.plugins)) byName.set(skill.name, skill);
  for (const dir of [dirs.bundled, dirs.user, join(cwd, ".suite", "skills")]) {
    for (const skill of readSkills(dir)) byName.set(skill.name, skill);
  }
  return [...byName.values()];
}

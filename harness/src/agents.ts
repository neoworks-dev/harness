// Agent definitions (grove#494): named subagents written as Claude Code writes them, a markdown file
// with frontmatter and the system prompt as its body.
//
//   ---
//   name: reviewer
//   description: Reviews a diff for correctness. Use after a change is made.
//   tools: Read, Grep, Bash
//   model: sonnet
//   ---
//   You review diffs…
//
// They come from `<cwd>/.suite/agents/*.md` (the project's), `~/.config/suite/agents/*.md` (the
// user's) and `agents/*.md` of each enabled plugin (plugins.ts). When a name is defined twice the
// project's wins over the user's, which wins over a plugin's. A plugin's agent is also known as
// `plugin:name`, which is how plugins refer to each other's.
//
// `delegate {agent: "reviewer", task}` runs neo with the body as its instructions, the tools the
// list allows (tool-names.ts maps Claude Code's names) and the model. `model:` is a neo model
// setting ("openrouter/deepseek/deepseek-v4-flash"), except Claude Code's own names: sonnet,
// opus, haiku, inherit and claude-* ids stand for whatever model the session has, because they
// name models neo has no account for. Not read: color, effort, maxTurns, skills, hooks and
// whatever else a Claude Code agent can carry.
import { readdirSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { enabledPlugins } from "./plugins.ts";
import { neoToolsFor, SEARCH_ONLY } from "./tool-names.ts";
import { isRecord } from "./util.ts";

export type AgentSource = "project" | "user" | "plugin";

export interface AgentDefinition {
  name: string;
  /** When to use it: what the main agent's prompt lists. */
  description: string;
  /** The body of the file: the agent's instructions. */
  prompt: string;
  /** Claude Code tool names as written; absent: all tools. */
  tools?: string[];
  disallowedTools?: string[];
  /** As written. See `agentModel`. */
  model?: string;
  source: AgentSource;
  path: string;
  /** The plugin's name (not its id), for a plugin's agent. */
  plugin?: string;
}

/** What other code needs to know of a definition without its prompt. */
export interface AgentInfo {
  name: string;
  description: string;
  /** A neo model setting; absent when the agent takes the session's model. */
  model?: string;
  source: AgentSource;
  /** The plugin's name, for a plugin's agent (`plugin:name` also finds it). */
  plugin?: string;
}

/** A description longer than this is cut: the prompt carries one for every agent. */
const DESCRIPTION_LIMIT = 400;

function unquote(value: string): string {
  const quote = value[0];
  if (quote === '"' && value.length >= 2 && value.endsWith('"')) {
    try {
      const parsed: unknown = JSON.parse(value);
      if (typeof parsed === "string") return parsed;
    } catch {
      // Not JSON escapes: the text between the quotes as it is.
    }
    return value.slice(1, -1);
  }
  if (quote === "'" && value.length >= 2 && value.endsWith("'")) {
    return value.slice(1, -1).replaceAll("''", "'");
  }
  return value;
}

/** A frontmatter value: its text, and its items when it was a list. */
interface Field {
  text: string;
  items?: string[];
}

/**
 * The part of YAML agent files use: `key: value` at the left edge, with the value on the line, a
 * `|` or `>` block under it, plain text continued on indented lines, or `- item` lines. Lenient
 * where YAML is strict (a description may hold colons), which is how Claude Code reads them.
 */
function parseFields(block: string): Map<string, Field> {
  const fields = new Map<string, Field>();
  const lines = block.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const match = /^([A-Za-z_][\w-]*):[ \t]*(.*)$/.exec(lines[i] ?? "");
    if (!match) continue;
    const key = match[1] ?? "";
    const inline = (match[2] ?? "").trim();
    const rest: string[] = [];
    while (i + 1 < lines.length) {
      const next = lines[i + 1] ?? "";
      if (next.trim() !== "" && !/^\s/.test(next) && !next.startsWith("- ")) break;
      rest.push(next);
      i++;
    }
    while (rest.at(-1)?.trim() === "") rest.pop();
    const indent = Math.min(
      ...rest
        .filter((line) => line.trim() !== "")
        .map((line) => /^\s*/.exec(line)?.[0].length ?? 0),
    );
    const dedented = rest.map((line) => line.slice(Number.isFinite(indent) ? indent : 0));
    if (/^[|>][+-]?$/.test(inline)) {
      const folded = inline.startsWith(">");
      const text = folded
        ? dedented
            .join("\n")
            .split(/\n{2,}/)
            .map((paragraph) => paragraph.replaceAll("\n", " "))
            .join("\n")
        : dedented.join("\n");
      fields.set(key, { text: text.trim() });
    } else if (inline === "" && dedented.some((line) => line.startsWith("- "))) {
      const items = dedented
        .filter((line) => line.startsWith("- "))
        .map((line) => unquote(line.slice(2).trim()));
      fields.set(key, { text: items.join(", "), items });
    } else {
      const text = [inline, ...dedented.map((line) => line.trim())].filter(Boolean).join(" ");
      fields.set(key, { text: unquote(text) });
    }
  }
  return fields;
}

/** `Read, Grep` or `["Read", "Grep"]` or a YAML list; commas inside parentheses stay together. */
function toolList(field: Field | undefined): string[] | undefined {
  if (!field) return undefined;
  if (field.items) return field.items;
  let text = field.text.trim();
  if (text.startsWith("[") && text.endsWith("]")) {
    try {
      const parsed: unknown = JSON.parse(text);
      if (Array.isArray(parsed)) return parsed.filter((item) => typeof item === "string");
    } catch {
      text = text.slice(1, -1);
    }
  }
  const items: string[] = [];
  let depth = 0;
  let current = "";
  for (const char of text) {
    if (char === "(") depth++;
    else if (char === ")") depth = Math.max(0, depth - 1);
    if (char === "," && depth === 0) {
      items.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  items.push(current);
  return items.map((item) => unquote(item.trim())).filter((item) => item !== "");
}

/** What the main agent reads of a description: the first part, before the examples. */
function shortDescription(text: string): string {
  const cut = text.split(/<example|\n\s*Examples?:/i)[0] ?? text;
  const flat = cut.replace(/\s+/g, " ").trim();
  return flat.length > DESCRIPTION_LIMIT ? `${flat.slice(0, DESCRIPTION_LIMIT - 1)}…` : flat;
}

/**
 * An agent file: frontmatter, then the body. The name defaults to `fallbackName` (the file's).
 * Undefined without frontmatter, without a description or without a body.
 */
export function parseAgent(
  text: string,
  fallbackName: string,
): Omit<AgentDefinition, "source" | "path" | "plugin"> | undefined {
  const match = /^---[ \t]*\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  if (!match?.[1]) return undefined;
  const fields = parseFields(match[1]);
  const description = shortDescription(fields.get("description")?.text ?? "");
  const prompt = text.slice(match[0].length).trim();
  if (description === "" || prompt === "") return undefined;
  const agent: Omit<AgentDefinition, "source" | "path" | "plugin"> = {
    name: fields.get("name")?.text || fallbackName,
    description,
    prompt,
  };
  const tools = toolList(fields.get("tools"));
  if (tools) agent.tools = tools;
  const disallowed = toolList(fields.get("disallowedTools"));
  if (disallowed) agent.disallowedTools = disallowed;
  const model = fields.get("model")?.text;
  if (model) agent.model = model;
  return agent;
}

function readAgents(dir: string, source: AgentSource, plugin?: string): AgentDefinition[] {
  let entries: string[];
  try {
    entries = readdirSync(dir).sort();
  } catch {
    return [];
  }
  const agents: AgentDefinition[] = [];
  for (const entry of entries) {
    if (!entry.endsWith(".md")) continue;
    const path = join(dir, entry);
    let text: string;
    try {
      text = readFileSync(path, "utf8");
    } catch {
      continue;
    }
    const parsed = parseAgent(text, basename(entry, ".md"));
    if (!parsed) continue;
    const agent: AgentDefinition = { ...parsed, source, path };
    if (plugin) agent.plugin = plugin;
    agents.push(agent);
  }
  return agents;
}

/** Folders and files a plugin's manifest adds to `agents/`. */
function manifestAgentPaths(root: string): string[] {
  let manifest: unknown;
  try {
    manifest = JSON.parse(readFileSync(join(root, ".claude-plugin", "plugin.json"), "utf8"));
  } catch {
    return [];
  }
  if (!isRecord(manifest)) return [];
  const declared = manifest.agents;
  const list = Array.isArray(declared) ? declared : [declared];
  return list
    .filter((entry): entry is string => typeof entry === "string")
    .map((p) => resolve(root, p));
}

export interface AgentDirs {
  /** The user's folder of agents (~/.config/suite/agents). */
  user?: string;
  /** Where plugins are installed (HarnessConfig.plugins). */
  plugins?: string;
}

/**
 * Every definition a session in `cwd` knows, the project's first, then the user's, then the
 * plugins' (by plugin id). `findAgent` takes the first that fits, so earlier ones win.
 */
export function loadAgents(cwd: string, dirs: AgentDirs): AgentDefinition[] {
  const all = readAgents(join(cwd, ".suite", "agents"), "project");
  if (dirs.user) all.push(...readAgents(dirs.user, "user"));
  for (const plugin of enabledPlugins(dirs.plugins)) {
    all.push(...readAgents(join(plugin.root, "agents"), "plugin", plugin.name));
    for (const extra of manifestAgentPaths(plugin.root)) {
      if (extra.endsWith(".md")) {
        const [dir, file] = [join(extra, ".."), basename(extra)];
        all.push(
          ...readAgents(dir, "plugin", plugin.name).filter((a) => basename(a.path) === file),
        );
      } else {
        all.push(...readAgents(extra, "plugin", plugin.name));
      }
    }
  }
  return all;
}

/** The definition a name asks for: `name` (project, user, then plugins) or `plugin:name`. */
export function findAgent<T extends { name: string; plugin?: string }>(
  agents: readonly T[],
  name: string,
): T | undefined {
  const wanted = name.trim().toLowerCase();
  const colon = wanted.indexOf(":");
  if (colon > 0) {
    const plugin = wanted.slice(0, colon);
    const own = wanted.slice(colon + 1);
    return agents.find(
      (agent) => agent.plugin?.toLowerCase() === plugin && agent.name.toLowerCase() === own,
    );
  }
  return agents.find((agent) => agent.name.toLowerCase() === wanted);
}

/** One definition per name, the one `findAgent` finds: what is listed to the agent. */
export function listAgents<T extends { name: string }>(agents: readonly T[]): T[] {
  const seen = new Set<string>();
  return agents.filter((agent) => {
    const key = agent.name.toLowerCase();
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/**
 * The neo model setting an agent runs with, if it names one. Claude Code's own aliases (sonnet,
 * opus, haiku, inherit, default) and claude-* ids are models neo has no account for; they stand
 * for the session's model, so nothing is set.
 */
export function agentModel(agent: AgentDefinition): string | undefined {
  const model = agent.model?.trim();
  if (!model) return undefined;
  const lower = model.toLowerCase();
  if (["sonnet", "opus", "haiku", "inherit", "default"].includes(lower)) return undefined;
  if (lower.startsWith("claude-")) return undefined;
  return model;
}

export function agentInfo(agent: AgentDefinition): AgentInfo {
  const info: AgentInfo = {
    name: agent.name,
    description: agent.description,
    source: agent.source,
  };
  const model = agentModel(agent);
  if (model) info.model = model;
  if (agent.plugin) info.plugin = agent.plugin;
  return info;
}

/** Neo tools that go with the Claude Code name in disallowedTools (edit leaves read alone). */
function neoToolsToRemove(entry: string): string[] {
  const bare = entry.trim().replace(/\(.*$/s, "").toLowerCase();
  if (SEARCH_ONLY.has(bare)) return [];
  if (bare === "edit" || bare === "multiedit") return ["edit"];
  return neoToolsFor(entry);
}

/** Whether a neo tool is one the agent may use; all of them without a `tools` list. */
export function toolFilter(agent: AgentDefinition): (neoTool: string) => boolean {
  const allowed = agent.tools ? new Set(agent.tools.flatMap(neoToolsFor)) : undefined;
  const denied = new Set((agent.disallowedTools ?? []).flatMap(neoToolsToRemove));
  return (tool) => (allowed === undefined || allowed.has(tool)) && !denied.has(tool);
}

/** An agent whose shell is there to search (Grep, Glob, LS) and not to change anything. */
export function shellIsForLooking(agent: AgentDefinition): boolean {
  if (!agent.tools) return false;
  const bare = agent.tools.map((entry) => entry.trim().replace(/\(.*$/s, "").toLowerCase());
  const searches = bare.some((entry) => SEARCH_ONLY.has(entry));
  const changes = bare.some((entry) => ["bash", "write", "edit", "multiedit"].includes(entry));
  return searches && !changes;
}

/**
 * Neo's instructions with the agent's under them. The agent's tools may not all be there; what
 * it was told to use is.
 */
export function agentPrompt(base: string, agent: AgentDefinition): string {
  let prompt = `${base}

You are the "${agent.name}" agent, working on a task another agent delegated to you. Your instructions:

${agent.prompt}`;
  if (shellIsForLooking(agent)) {
    prompt +=
      "\n\nbash is there for you to look around (grep, find, ls, cat, git log and the like). " +
      "Do not use it to change anything.";
  }
  return prompt;
}

/** The section of the main agent's prompt that lists the agents it can delegate to. */
export function agentsPrompt(agents: readonly AgentDefinition[]): string {
  const lines = listAgents(agents).map((agent) => `- ${agent.name}: ${agent.description}`);
  if (lines.length === 0) return "";
  return `delegate also runs agents defined for this user and project: give their name as its
agent, instead of neo, claude or codex, and it runs neo with the instructions, tools and model
the definition gives. Delegate to one when its description fits the work.

${lines.join("\n")}`;
}

// Plugin commands (grove#495): `commands/<name>.md` in an enabled plugin is `/name`, a prompt
// template. The user types `/name args`; the agent gets the template with the arguments in it, the
// chat shows what was typed. `$ARGUMENTS` is all the arguments, `$1`…`$9` the words (quotes group
// words). Not done: `!`command`` output and `@file` includes in the body, `allowed-tools`.
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { declaredPaths, frontmatter } from "./plugin-parts.ts";
import { enabledPlugins, type Plugin } from "./plugins.ts";

export interface PluginCommand {
  /** What follows the slash: `commit`, or `plugin:commit` where the plain name is taken. */
  name: string;
  /** Always valid too: `plugin:commit`. */
  full: string;
  /** The plugin's id, `name@marketplace`. */
  plugin: string;
  description: string;
  /** What the arguments are, from `argument-hint`. */
  hint?: string;
  /** The prompt, with the placeholders. */
  template: string;
  /** The markdown file. */
  path: string;
  /** For `${CLAUDE_PLUGIN_ROOT}` in the template. */
  root: string;
}

const DESCRIPTION_LIMIT = 120;

function oneLine(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > DESCRIPTION_LIMIT ? `${flat.slice(0, DESCRIPTION_LIMIT - 1)}…` : flat;
}

/** The markdown files under a commands directory: `sub/name.md` is `sub:name`. */
function commandFiles(dir: string, prefix = ""): { name: string; path: string }[] {
  let entries: import("node:fs").Dirent[];
  try {
    entries = readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
      basename(a.name, ".md").localeCompare(basename(b.name, ".md")),
    );
  } catch {
    return [];
  }
  const out: { name: string; path: string }[] = [];
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...commandFiles(path, `${prefix}${entry.name}:`));
    else if (entry.name.endsWith(".md"))
      out.push({ name: `${prefix}${basename(entry.name, ".md")}`, path });
  }
  return out;
}

/** A command as read: its name in the plugin, and the rest. */
export interface Found {
  local: string;
  command: Omit<PluginCommand, "name">;
}

/** The commands of one plugin, by their names in it. */
export function readPluginCommands(plugin: Plugin): Found[] {
  const out: Found[] = [];
  const files: { name: string; path: string }[] = [];
  const dirs = [join(plugin.root, "commands"), ...declaredPaths(plugin, "commands")];
  for (const path of dirs) {
    if (path.endsWith(".md")) files.push({ name: basename(path, ".md"), path });
    else files.push(...commandFiles(path));
  }
  const seen = new Set<string>();
  for (const file of files) {
    if (seen.has(file.path)) continue;
    seen.add(file.path);
    let text: string;
    try {
      text = readFileSync(file.path, "utf8");
    } catch {
      continue;
    }
    const { fields, body } = frontmatter(text);
    if (body === "") continue;
    const first = body.split("\n").find((line) => line.trim() !== "") ?? "";
    const command: Omit<PluginCommand, "name"> = {
      full: `${plugin.name}:${file.name}`,
      plugin: plugin.id,
      description: oneLine(fields.get("description") || first.replace(/^#+\s*/, "")),
      template: body,
      path: file.path,
      root: plugin.root,
    };
    const hint = fields.get("argument-hint");
    if (hint) command.hint = hint;
    out.push({ local: file.name, command });
  }
  return out;
}

/**
 * The commands of the enabled plugins. A command is `/name` unless that name is taken, by
 * `reserved` (the daemon's own commands) or by another plugin's command: then it is
 * `/plugin:name`. `/plugin:name` works for every command.
 */
export function loadPluginCommands(
  pluginsDir: string | undefined,
  reserved: ReadonlySet<string> = new Set(),
): PluginCommand[] {
  if (!pluginsDir) return [];
  const all = enabledPlugins(pluginsDir).flatMap(readPluginCommands);
  const counts = new Map<string, number>();
  for (const { local } of all) counts.set(local, (counts.get(local) ?? 0) + 1);
  return all.map(({ local, command }) => {
    const taken = reserved.has(local) || (counts.get(local) ?? 0) > 1;
    return { ...command, name: taken ? command.full : local };
  });
}

/** The command a typed name means: its menu name, or `plugin:name`. */
export function findPluginCommand(
  commands: PluginCommand[],
  typed: string,
): PluginCommand | undefined {
  return (
    commands.find((command) => command.name === typed) ??
    commands.find((command) => command.full === typed)
  );
}

/** The words of an argument string; single or double quotes group. */
export function splitArguments(args: string): string[] {
  const words: string[] = [];
  const pattern = /"([^"]*)"|'([^']*)'|(\S+)/g;
  for (const match of args.matchAll(pattern)) {
    const [, double, single, bare] = match;
    if (double !== undefined) words.push(double);
    else if (single !== undefined) words.push(single);
    else words.push(bare ?? "");
  }
  return words;
}

/** The prompt for `/name args`: the template with the arguments in place. */
export function expandCommand(command: PluginCommand, args: string): string {
  const words = splitArguments(args);
  let used = false;
  let text = command.template.replace(/\$ARGUMENTS|\$([1-9])/g, (_, digit?: string) => {
    used = true;
    return digit === undefined ? args : (words[Number(digit) - 1] ?? "");
  });
  text = text.replaceAll("${CLAUDE_PLUGIN_ROOT}", command.root);
  if (!used && args !== "") text += `\n\nARGUMENTS: ${args}`;
  return text;
}

/**
 * What a typed `/name args` becomes, if `name` is a plugin command: the prompt for the agent and
 * what the chat shows (the typed text). Undefined for anything else.
 */
export function pluginPrompt(
  commands: PluginCommand[],
  typed: string,
): { text: string; shown: string } | undefined {
  const match = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(typed.trim());
  if (!match?.[1]) return undefined;
  const command = findPluginCommand(commands, match[1]);
  if (!command) return undefined;
  return { text: expandCommand(command, (match[2] ?? "").trim()), shown: typed.trim() };
}

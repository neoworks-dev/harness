// Claude Code plugins, as the harness installs them (#25). The daemon's installer writes
// `<dir>/installed.json`; everything that uses a plugin's parts (skills, commands, agents, hooks,
// MCP servers, language servers) reads the enabled ones from here.
//
//   ~/.config/neo/plugins/
//     installed.json                 { version: 1, plugins: { "name@marketplace": InstalledPlugin } }
//     marketplaces.json              the marketplaces added (the installer's own)
//     cache/<marketplace>/<name>/<version>/   a plugin's folder: .claude-plugin/plugin.json, skills/, …
import { readFileSync } from "node:fs";
import { isAbsolute } from "node:path";
import { isRecord } from "./util.ts";

export interface InstalledPlugin {
  /** Absolute path of the plugin's folder. */
  root: string;
  version: string;
  enabled: boolean;
  /** ISO time it was installed or updated. */
  installedAt: string;
  /** What the marketplace entry adds beyond the folder, e.g. its `lspServers`. */
  entry?: Record<string, unknown>;
}

export interface InstalledFile {
  version: 1;
  plugins: Record<string, InstalledPlugin>;
}

/** An enabled plugin, ready to read parts from. */
export interface Plugin {
  /** `name@marketplace`. */
  id: string;
  name: string;
  marketplace: string;
  root: string;
  version: string;
  entry?: Record<string, unknown>;
}

/** What `installed.json` holds; empty when it is missing or broken. */
export function readInstalled(dir: string): InstalledFile {
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(`${dir}/installed.json`, "utf8"));
  } catch {
    return { version: 1, plugins: {} };
  }
  const plugins: Record<string, InstalledPlugin> = {};
  if (isRecord(parsed) && isRecord(parsed.plugins)) {
    for (const [id, value] of Object.entries(parsed.plugins)) {
      // The installer always writes these; an entry without them is broken.
      if (
        !isRecord(value) ||
        typeof value.root !== "string" ||
        !isAbsolute(value.root) ||
        typeof value.version !== "string" ||
        typeof value.installedAt !== "string"
      ) {
        continue;
      }
      const plugin: InstalledPlugin = {
        root: value.root,
        version: value.version,
        enabled: value.enabled !== false,
        installedAt: value.installedAt,
      };
      if (isRecord(value.entry)) plugin.entry = value.entry;
      plugins[id] = plugin;
    }
  }
  return { version: 1, plugins };
}

/** The enabled plugins, by id. A broken entry (no absolute root, version or install time) is skipped. */
export function enabledPlugins(dir: string | undefined): Plugin[] {
  if (!dir) return [];
  const out: Plugin[] = [];
  for (const [id, plugin] of Object.entries(readInstalled(dir).plugins)) {
    const at = id.lastIndexOf("@");
    if (!plugin.enabled || at <= 0) continue;
    const found: Plugin = {
      id,
      name: id.slice(0, at),
      marketplace: id.slice(at + 1),
      root: plugin.root,
      version: plugin.version,
    };
    if (plugin.entry) found.entry = plugin.entry;
    out.push(found);
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** `${CLAUDE_PLUGIN_ROOT}` in a hook command or an MCP server's command, args and env. */
export function expandPluginRoot(text: string, plugin: Plugin): string {
  return text.replaceAll("${CLAUDE_PLUGIN_ROOT}", plugin.root);
}

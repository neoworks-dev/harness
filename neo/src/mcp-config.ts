// MCP servers the daemon runs for neo (#22), in Claude Code's `mcpServers` shape:
//
//   { "mcpServers": { "headroom": { "command": "headroom", "args": ["mcp", "serve"], "env": {} },
//                     "docs": { "type": "http", "url": "https://…", "headers": {} } } }
//
// They come from `mcpServers` in ~/.config/suite/config.json, from an enabled plugin's `.mcp.json`
// (with `${CLAUDE_PLUGIN_ROOT}` expanded), and, when config.json says `"projectMcp": true`, from
// `<cwd>/.mcp.json` of the user's sessions. A project's file starts programs of whoever wrote the
// repository, so it is off until the user turns it on. The first source to name an app wins.
import { homedir } from "node:os";
import { join } from "node:path";
import { declaredPaths, readJsonObject, readManifest } from "./plugin-parts.ts";
import { enabledPlugins, expandPluginRoot, type Plugin } from "./plugins.ts";
import { isRecord } from "./util.ts";

export type McpServerConfig =
  | { type: "stdio"; command: string; args: string[]; env: Record<string, string>; cwd?: string }
  | { type: "http"; url: string; headers: Record<string, string> };

/** A server to run, named by the app id its tools appear under. */
export interface McpServerSpec {
  /** `mcp-headroom`; matches the app id pattern, so it can't clash with an app's. */
  app: string;
  /** The name it was configured under. */
  name: string;
  config: McpServerConfig;
  /** Where it was configured: "config.json", a plugin's id or a project's .mcp.json. */
  origin: string;
}

export interface McpSources {
  /** ~/.config/suite/config.json. */
  configFile?: string;
  /** The plugins directory (HarnessConfig.plugins). */
  plugins?: string;
  /** Session directories whose `.mcp.json` counts, if config.json allows projects. */
  projects?: string[];
  /** For `${VAR}` in the configuration. Default: the process's. */
  env?: Record<string, string | undefined>;
}

export interface LoadedMcp {
  servers: McpServerSpec[];
  /** What was skipped, in words for a log. */
  problems: string[];
}

/** Where the suite keeps its configuration: $XDG_CONFIG_HOME/suite. */
export function suiteConfigDir(env: Record<string, string | undefined> = process.env): string {
  return join(env.XDG_CONFIG_HOME || join(env.HOME || homedir(), ".config"), "suite");
}

/** `${VAR}` and `${VAR:-default}`, from `env`; an unset variable without a default is empty. */
export function expandEnv(text: string, env: Record<string, string | undefined>): string {
  return text.replace(
    /\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}/g,
    (_, name: string, fallback?: string) => {
      const value = env[name];
      return value !== undefined && value !== "" ? value : (fallback ?? "");
    },
  );
}

/** An app id for a server name: `mcp-` and the name, lower-case, in the id pattern; or undefined. */
export function appIdFor(...parts: string[]): string | undefined {
  const slug = ["mcp", ...parts]
    .join("-")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32)
    .replace(/-+$/, "");
  return /^[a-z][a-z0-9-]{0,31}$/.test(slug) && slug !== "mcp" ? slug : undefined;
}

function strings(value: unknown): string[] | undefined {
  if (value === undefined) return [];
  return Array.isArray(value) && value.every((item) => typeof item === "string")
    ? value
    : undefined;
}

function stringMap(value: unknown): Record<string, string> | undefined {
  if (value === undefined) return {};
  if (!isRecord(value)) return undefined;
  const out: Record<string, string> = {};
  for (const [key, item] of Object.entries(value)) {
    if (typeof item !== "string") return undefined;
    out[key] = item;
  }
  return out;
}

/**
 * One server entry. `expand` is applied to every string (command, args, env values, url, headers).
 * A string says why it can't be used.
 */
export function parseServer(
  value: unknown,
  expand: (text: string) => string,
): McpServerConfig | string {
  if (!isRecord(value)) return "not an object";
  const type = value.type;
  if (type === "http" || (type === undefined && typeof value.url === "string")) {
    const headers = stringMap(value.headers);
    if (typeof value.url !== "string" || headers === undefined)
      return "needs a url and string headers";
    return {
      type: "http",
      url: expand(value.url),
      headers: Object.fromEntries(Object.entries(headers).map(([k, v]) => [k, expand(v)])),
    };
  }
  if (type !== undefined && type !== "stdio") {
    return `type ${JSON.stringify(type)} isn't supported (stdio and http are)`;
  }
  const args = strings(value.args);
  const env = stringMap(value.env);
  if (
    typeof value.command !== "string" ||
    value.command === "" ||
    args === undefined ||
    env === undefined
  ) {
    return "needs a command, with args as a list of strings and env as strings";
  }
  const config: McpServerConfig = {
    type: "stdio",
    command: expand(value.command),
    args: args.map(expand),
    env: Object.fromEntries(Object.entries(env).map(([k, v]) => [k, expand(v)])),
  };
  if (typeof value.cwd === "string") config.cwd = expand(value.cwd);
  return config;
}

/** The server table of a `.mcp.json` or config.json: `{ mcpServers: {…} }`, or the table itself. */
function serverTable(file: Record<string, unknown>): Record<string, unknown> {
  return isRecord(file.mcpServers) ? file.mcpServers : file;
}

class Collector {
  readonly servers: McpServerSpec[] = [];
  readonly problems: string[] = [];

  constructor(private readonly env: Record<string, string | undefined>) {}

  /** Adds the table's servers, named by `id(name)`; the first of an app id stays. */
  add(
    table: Record<string, unknown>,
    origin: string,
    id: (name: string) => string | undefined,
    expand?: (text: string) => string,
  ): void {
    const expandAll = (text: string): string => expandEnv(expand ? expand(text) : text, this.env);
    for (const [name, value] of Object.entries(table)) {
      const app = id(name);
      if (app === undefined) {
        this.problems.push(`${origin}: "${name}" can't be named as an app`);
        continue;
      }
      const config = parseServer(value, expandAll);
      if (typeof config === "string") {
        this.problems.push(`${origin}: ${name}: ${config}`);
        continue;
      }
      const existing = this.servers.find((server) => server.app === app);
      if (existing) {
        if (JSON.stringify(existing.config) !== JSON.stringify(config)) {
          this.problems.push(`${origin}: ${name} is ${app} already (from ${existing.origin})`);
        }
        continue;
      }
      this.servers.push({ app, name, config, origin });
    }
  }
}

/** The servers an enabled plugin declares: its `.mcp.json`, and `mcpServers` in manifest or entry. */
function pluginServers(plugin: Plugin): Record<string, unknown> {
  const table: Record<string, unknown> = {};
  const file = readJsonObject(join(plugin.root, ".mcp.json"));
  if (file) Object.assign(table, serverTable(file));
  for (const source of [readManifest(plugin.root), plugin.entry ?? {}]) {
    if (isRecord(source.mcpServers)) Object.assign(table, source.mcpServers);
  }
  for (const path of declaredPaths(plugin, "mcpServers")) {
    const extra = readJsonObject(path);
    if (extra) Object.assign(table, serverTable(extra));
  }
  return table;
}

/** Whether config.json lets sessions' directories bring their own `.mcp.json`. */
export function projectMcpAllowed(configFile: string | undefined): boolean {
  return configFile !== undefined && readJsonObject(configFile)?.projectMcp === true;
}

/** Every server to run now, from the sources in the order above. */
export function loadMcpServers(sources: McpSources): LoadedMcp {
  const env = sources.env ?? process.env;
  const collect = new Collector(env);
  const config = sources.configFile ? readJsonObject(sources.configFile) : undefined;
  if (config && isRecord(config.mcpServers)) {
    collect.add(config.mcpServers, "config.json", (name) => appIdFor(name));
  }
  if (config?.projectMcp === true) {
    for (const dir of [...new Set(sources.projects ?? [])]) {
      const file = readJsonObject(join(dir, ".mcp.json"));
      if (!file) continue;
      collect.add(serverTable(file), join(dir, ".mcp.json"), (name) => appIdFor(name));
    }
  }
  for (const plugin of enabledPlugins(sources.plugins)) {
    const table = pluginServers(plugin);
    collect.add(
      table,
      plugin.id,
      (name) => (name === plugin.name ? appIdFor(name) : appIdFor(plugin.name, name)),
      (text) => expandPluginRoot(text, plugin),
    );
  }
  return { servers: collect.servers, problems: collect.problems };
}

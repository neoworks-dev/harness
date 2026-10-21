// Marketplaces and the installer (grove#495). A marketplace is a git repository (or a folder)
// with `.claude-plugin/marketplace.json`; the harness keeps its own copy of each and installs
// plugins from them itself, so it doesn't depend on Claude Code being installed.
//
//   <dir>/marketplaces.json              { version: 1, marketplaces: { name: MarketplaceRecord } }
//   <dir>/marketplaces/<name>/           the marketplace's repository
//   <dir>/installed.json                 what is installed (plugins.ts)
//   <dir>/cache/<marketplace>/<name>/<version>/   an installed plugin
//
// An entry's `source` is a path in the marketplace ("./plugins/x"), or {source: "github", repo},
// {source: "url", url}, {source: "git-subdir", url, path}; each may pin `ref` and `sha`. npm
// sources are not supported.
import { execFile } from "node:child_process";
import {
  cp,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { readPluginCommands } from "./plugin-commands.ts";
import { pluginLspServers } from "./plugin-lsp.ts";
import { inside, readJsonObject, readManifest } from "./plugin-parts.ts";
import { type Plugin, readInstalled } from "./plugins.ts";
import { readPluginSkills } from "./skills.ts";
import { isRecord } from "./util.ts";

/** Where a marketplace was added from. */
export type MarketplaceSource =
  | { source: "github"; repo: string }
  | { source: "git"; url: string }
  | { source: "path"; path: string };

export interface MarketplaceRecord {
  name: string;
  from: MarketplaceSource;
  /** Absolute path of the copy. */
  location: string;
  /** The commit it was at when copied, for a git source. */
  commit?: string;
  addedAt: string;
}

/** One entry of a marketplace's `plugins` list. */
export interface MarketplaceEntry {
  name: string;
  description?: string;
  [key: string]: unknown;
}

/** What an installed plugin brings, for the reply to the user. */
export interface PluginParts {
  skills: number;
  commands: number;
  mcpServers: number;
  lspServers: number;
  agents: number;
  hooks: boolean;
}

export interface InstallResult {
  id: string;
  version: string;
  root: string;
  parts: PluginParts;
}

/** A failure the user can read and act on. */
export class PluginError extends Error {}

const GIT_TIMEOUT_MS = 5 * 60_000;

function run(program: string, args: string[], cwd?: string): Promise<string> {
  return new Promise((resolvePromise, reject) => {
    execFile(
      program,
      args,
      {
        cwd,
        timeout: GIT_TIMEOUT_MS,
        maxBuffer: 16 * 1024 * 1024,
        env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
      },
      (error, stdout, stderr) => {
        if (!error) {
          resolvePromise(stdout.trim());
          return;
        }
        const detail = stderr.trim().split("\n").slice(-2).join(" ");
        const missing = "code" in error && error.code === "ENOENT";
        reject(
          new PluginError(
            missing
              ? `${program} isn't installed`
              : `${program} ${args[0] ?? ""} failed: ${detail || error.message}`,
          ),
        );
      },
    );
  });
}

const git = (args: string[], cwd?: string): Promise<string> => run("git", args, cwd);

function now(): string {
  return new Date().toISOString();
}

/** A name that is safe as a directory name: it comes from files we don't control. */
function segment(value: string, what: string): string {
  if (!/^[A-Za-z0-9][A-Za-z0-9._+-]*$/.test(value) || value.includes("..")) {
    throw new PluginError(`${what} "${value}" isn't a usable name`);
  }
  return value;
}

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function writeJson(path: string, value: unknown): Promise<void> {
  const tmp = `${path}.${process.pid}.tmp`;
  await writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`);
  await rename(tmp, path);
}

// ---- marketplaces ----

interface MarketplacesFile {
  version: 1;
  marketplaces: Record<string, MarketplaceRecord>;
}

export function readMarketplaces(dir: string): MarketplacesFile {
  const file = readJsonObject(join(dir, "marketplaces.json"));
  const marketplaces: Record<string, MarketplaceRecord> = {};
  if (file && isRecord(file.marketplaces)) {
    for (const [name, value] of Object.entries(file.marketplaces)) {
      if (!isRecord(value) || typeof value.location !== "string" || !isRecord(value.from)) continue;
      marketplaces[name] = { ...(value as unknown as MarketplaceRecord), name };
    }
  }
  return { version: 1, marketplaces };
}

/** How the user's text names a marketplace: `owner/repo`, a git URL, or a path. */
export function parseSource(input: string, cwd = process.cwd()): MarketplaceSource {
  const text = input.trim();
  if (text === "") throw new PluginError("which marketplace? Give owner/repo, a git URL or a path");
  if (/^(https?:\/\/|ssh:\/\/|git:\/\/|file:\/\/|git@)/.test(text))
    return { source: "git", url: text };
  const asPath = text.startsWith("~/") ? join(homedir(), text.slice(2)) : resolve(cwd, text);
  if (/^[\w.-]+\/[\w.-]+$/.test(text) && !text.startsWith(".")) {
    return { source: "github", repo: text.replace(/\.git$/, "") };
  }
  return { source: "path", path: asPath };
}

/** The source as the user would type it. */
function sourceText(source: MarketplaceSource): string {
  if (source.source === "path") return source.path;
  if (source.source === "github") return source.repo;
  return source.url;
}

function gitUrl(source: MarketplaceSource): string | undefined {
  if (source.source === "github") return `https://github.com/${source.repo}.git`;
  if (source.source === "git") return source.url;
  return undefined;
}

interface Fetch {
  ref?: string;
  sha?: string;
}

/** Clones `url` into `dest` (an empty or missing directory), at `ref`, pinned to `sha` if given. */
async function clone(url: string, dest: string, { ref, sha }: Fetch): Promise<void> {
  if (sha) {
    try {
      await mkdir(dest, { recursive: true });
      await git(["init", "-q"], dest);
      await git(["remote", "add", "origin", url], dest);
      await git(["fetch", "-q", "--depth", "1", "origin", sha], dest);
      await git(["checkout", "-q", "--detach", "FETCH_HEAD"], dest);
      return;
    } catch {
      // A server may not hand out a commit that is not a tip: clone it all and check it out.
      await rm(dest, { recursive: true, force: true });
    }
    await git(["clone", "-q", url, dest]);
    await git(["checkout", "-q", "--detach", sha], dest);
    return;
  }
  const args = ["clone", "-q", "--depth", "1"];
  if (ref) args.push("--branch", ref);
  await git([...args, url, dest]);
}

async function readMarketplaceFile(location: string): Promise<Record<string, unknown>> {
  const path = join(location, ".claude-plugin", "marketplace.json");
  let parsed: unknown;
  try {
    parsed = JSON.parse(await readFile(path, "utf8"));
  } catch (err) {
    const missing = err instanceof Error && "code" in err && err.code === "ENOENT";
    throw new PluginError(
      missing ? "no .claude-plugin/marketplace.json there" : "marketplace.json isn't valid JSON",
    );
  }
  if (!isRecord(parsed) || typeof parsed.name !== "string" || !Array.isArray(parsed.plugins)) {
    throw new PluginError("marketplace.json needs a name and a plugins list");
  }
  return parsed;
}

/** The entries of an added marketplace. */
export async function marketplaceEntries(record: MarketplaceRecord): Promise<MarketplaceEntry[]> {
  const file = await readMarketplaceFile(record.location);
  const entries: MarketplaceEntry[] = [];
  for (const item of file.plugins as unknown[]) {
    if (isRecord(item) && typeof item.name === "string") entries.push(item as MarketplaceEntry);
  }
  return entries;
}

async function headCommit(location: string): Promise<string | undefined> {
  try {
    return await git(["rev-parse", "HEAD"], location);
  } catch {
    return undefined;
  }
}

/** Puts a copy of the marketplace at `dest`; the copy of a folder leaves out its `.git`. */
async function fetchMarketplace(from: MarketplaceSource, dest: string): Promise<void> {
  const url = gitUrl(from);
  if (url) {
    await clone(url, dest, {});
    return;
  }
  if (from.source !== "path") return;
  if (!(await exists(from.path))) throw new PluginError(`${from.path} doesn't exist`);
  await cp(from.path, dest, {
    recursive: true,
    filter: (path) => !path.split("/").includes("node_modules"),
  });
}

/**
 * Adds a marketplace: copies it into `<dir>/marketplaces/<name>` (named by its marketplace.json)
 * and records it. Adding one again replaces the copy.
 */
export async function addMarketplace(
  dir: string,
  input: string,
  cwd?: string,
): Promise<{ record: MarketplaceRecord; plugins: number }> {
  const from = parseSource(input, cwd);
  await mkdir(join(dir, "marketplaces"), { recursive: true });
  const temp = await mkdtemp(join(dir, "marketplaces", ".adding-"));
  try {
    await fetchMarketplace(from, temp);
    const file = await readMarketplaceFile(temp);
    const name = segment(String(file.name), "marketplace");
    const location = join(dir, "marketplaces", name);
    await rm(location, { recursive: true, force: true });
    await rename(temp, location);
    const record: MarketplaceRecord = { name, from, location, addedAt: now() };
    const commit = await headCommit(location);
    if (commit) record.commit = commit;
    const known = readMarketplaces(dir);
    known.marketplaces[name] = record;
    await writeJson(join(dir, "marketplaces.json"), known);
    return { record, plugins: (file.plugins as unknown[]).length };
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
}

/** Fetches a marketplace's source again (all of them without a name). Returns the names updated. */
export async function updateMarketplaces(dir: string, name?: string): Promise<string[]> {
  const known = readMarketplaces(dir);
  const names = name === undefined ? Object.keys(known.marketplaces) : [name];
  for (const each of names) {
    if (!known.marketplaces[each]) throw new PluginError(`no marketplace named ${each}`);
  }
  for (const each of names) {
    const record = known.marketplaces[each];
    if (!record) continue;
    const input = sourceText(record.from);
    const added = await addMarketplace(dir, input);
    if (added.record.name !== each) {
      throw new PluginError(`${each} is called ${added.record.name} now`);
    }
  }
  return names;
}

/**
 * Removes a marketplace and the plugins installed from it. Returns the ids of those plugins.
 */
export async function removeMarketplace(dir: string, name: string): Promise<string[]> {
  const known = readMarketplaces(dir);
  const record = known.marketplaces[name];
  if (!record) throw new PluginError(`no marketplace named ${name}`);
  const removed: string[] = [];
  for (const id of Object.keys(readInstalled(dir).plugins)) {
    if (id.endsWith(`@${name}`)) {
      await uninstallPlugin(dir, id);
      removed.push(id);
    }
  }
  delete known.marketplaces[name];
  await writeJson(join(dir, "marketplaces.json"), known);
  if (inside(join(dir, "marketplaces"), record.location)) {
    await rm(record.location, { recursive: true, force: true });
  }
  return removed;
}

// ---- installing ----

/** `name@marketplace`; the marketplace may be left out when only one has the plugin. */
function splitId(id: string): { name: string; marketplace?: string } {
  const at = id.lastIndexOf("@");
  if (at <= 0) return { name: id };
  return { name: id.slice(0, at), marketplace: id.slice(at + 1) };
}

async function findEntry(
  dir: string,
  wanted: string,
): Promise<{ record: MarketplaceRecord; entry: MarketplaceEntry }> {
  const { name, marketplace } = splitId(wanted);
  const known = readMarketplaces(dir).marketplaces;
  const records = marketplace === undefined ? Object.values(known) : [known[marketplace]];
  if (marketplace !== undefined && !known[marketplace]) {
    throw new PluginError(
      `no marketplace named ${marketplace}: add it with /plugin marketplace add`,
    );
  }
  const found: { record: MarketplaceRecord; entry: MarketplaceEntry }[] = [];
  for (const record of records) {
    if (!record) continue;
    const entries = await marketplaceEntries(record);
    let entry = entries.find((candidate) => candidate.name === name);
    if (!entry) {
      // A marketplace may have renamed the plugin.
      const renames = (await readMarketplaceFile(record.location)).renames;
      const renamed = isRecord(renames) ? renames[name] : undefined;
      if (typeof renamed === "string")
        entry = entries.find((candidate) => candidate.name === renamed);
    }
    if (entry) found.push({ record, entry });
  }
  const first = found[0];
  if (!first) {
    const where = marketplace === undefined ? "any marketplace" : marketplace;
    throw new PluginError(`no plugin ${name} in ${where}`);
  }
  if (found.length > 1) {
    const options = found.map(({ record }) => `${name}@${record.name}`).join(", ");
    throw new PluginError(`${name} is in several marketplaces: ${options}`);
  }
  return first;
}

/** Copies the plugin an entry points at into `dest`. Returns the commit it is at, if known. */
async function fetchPlugin(
  record: MarketplaceRecord,
  entry: MarketplaceEntry,
  dest: string,
): Promise<string | undefined> {
  const source = entry.source;
  if (typeof source === "string") {
    const marketplaceFile = await readMarketplaceFile(record.location);
    const pluginRoot = isRecord(marketplaceFile.metadata)
      ? marketplaceFile.metadata.pluginRoot
      : undefined;
    let base = record.location;
    if (typeof pluginRoot === "string") base = resolve(record.location, pluginRoot);
    const from = resolve(base, source);
    if (!inside(record.location, from) || !(await exists(from))) {
      throw new PluginError(`${entry.name}: its source ${source} isn't in the marketplace`);
    }
    await cp(from, dest, { recursive: true });
    return record.commit;
  }
  if (!isRecord(source)) throw new PluginError(`${entry.name}: it has no source`);
  const kind = source.source;
  let url: string | undefined;
  if (kind === "github" && typeof source.repo === "string")
    url = `https://github.com/${source.repo}.git`;
  else if (
    (kind === "url" || kind === "git" || kind === "git-subdir") &&
    typeof source.url === "string"
  ) {
    url = source.url;
  }
  if (url === undefined)
    throw new PluginError(`${entry.name}: ${String(kind)} sources aren't supported`);
  const fetch: Fetch = {};
  if (typeof source.ref === "string") fetch.ref = source.ref;
  if (typeof source.sha === "string") fetch.sha = source.sha;
  const checkout = await mkdtemp(join(dest, "..", ".clone-"));
  try {
    await clone(url, checkout, fetch);
    const commit = await headCommit(checkout);
    let subdir = checkout;
    if (typeof source.path === "string" && source.path !== "") {
      subdir = resolve(checkout, source.path);
      if (!inside(checkout, subdir) || !(await exists(subdir))) {
        throw new PluginError(`${entry.name}: ${source.path} isn't in ${url}`);
      }
    }
    await cp(subdir, dest, {
      recursive: true,
      filter: (path) => !path.split("/").includes(".git"),
    });
    return commit;
  } finally {
    await rm(checkout, { recursive: true, force: true });
  }
}

function version(
  entry: MarketplaceEntry,
  manifest: Record<string, unknown>,
  commit: string | undefined,
): string {
  for (const candidate of [manifest.version, entry.version]) {
    if (typeof candidate === "string" && candidate !== "") return candidate;
  }
  return commit ? commit.slice(0, 12) : "unknown";
}

async function countDir(dir: string, accept: (name: string) => boolean): Promise<number> {
  try {
    return (await readdir(dir)).filter(accept).length;
  } catch {
    return 0;
  }
}

/** What a plugin brings. */
export async function describePlugin(plugin: Plugin): Promise<PluginParts> {
  const mcp = readJsonObject(join(plugin.root, ".mcp.json"));
  let mcpServers = 0;
  if (mcp) mcpServers = Object.keys(isRecord(mcp.mcpServers) ? mcp.mcpServers : mcp).length;
  return {
    skills: readPluginSkills(plugin).length,
    commands: readPluginCommands(plugin).length,
    mcpServers,
    lspServers: pluginLspServers(plugin).length,
    agents: await countDir(join(plugin.root, "agents"), (name) => name.endsWith(".md")),
    hooks:
      (await exists(join(plugin.root, "hooks", "hooks.json"))) ||
      isRecord(readManifest(plugin.root).hooks),
  };
}

/** The raw installed.json, so fields this code doesn't know survive a write. */
function rawInstalled(dir: string): {
  plugins: Record<string, Record<string, unknown>>;
  rest: Record<string, unknown>;
} {
  const file = readJsonObject(join(dir, "installed.json")) ?? {};
  const { plugins, ...rest } = file;
  const out: Record<string, Record<string, unknown>> = {};
  if (isRecord(plugins)) {
    for (const [id, value] of Object.entries(plugins)) if (isRecord(value)) out[id] = value;
  }
  return { plugins: out, rest };
}

async function writeInstalled(
  dir: string,
  plugins: Record<string, Record<string, unknown>>,
  rest: Record<string, unknown>,
): Promise<void> {
  await mkdir(dir, { recursive: true });
  await writeJson(join(dir, "installed.json"), { ...rest, version: 1, plugins });
}

/**
 * Installs `name@marketplace` (or `name` when one marketplace has it): copies the plugin into
 * `cache/<marketplace>/<name>/<version>/` and records it, enabled, with its marketplace entry.
 * Installing it again updates it and keeps it enabled or disabled as it was.
 */
export async function installPlugin(dir: string, wanted: string): Promise<InstallResult> {
  const { record, entry } = await findEntry(dir, wanted);
  const name = segment(entry.name, "plugin");
  const id = `${name}@${record.name}`;
  const parent = join(dir, "cache", record.name, name);
  await mkdir(parent, { recursive: true });
  const staging = await mkdtemp(join(parent, ".installing-"));
  const copy = join(staging, "plugin");
  try {
    const commit = await fetchPlugin(record, entry, copy);
    const ver = segment(version(entry, readManifest(copy), commit), "version");
    const root = join(parent, ver);
    await rm(root, { recursive: true, force: true });
    await rename(copy, root);
    const { plugins, rest } = rawInstalled(dir);
    const before = plugins[id];
    const installed: Record<string, unknown> = {
      ...before,
      root,
      version: ver,
      enabled: before?.enabled !== false,
      installedAt: now(),
      entry,
    };
    plugins[id] = installed;
    await writeInstalled(dir, plugins, rest);
    // Older versions of this plugin are no longer used.
    for (const other of await readdir(parent)) {
      if (other !== ver && !other.startsWith(".")) {
        await rm(join(parent, other), { recursive: true, force: true });
      }
    }
    const plugin: Plugin = { id, name, marketplace: record.name, root, version: ver, entry };
    return { id, version: ver, root, parts: await describePlugin(plugin) };
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

/** Removes an installed plugin and its files. */
export async function uninstallPlugin(dir: string, id: string): Promise<void> {
  const { plugins, rest } = rawInstalled(dir);
  const found = plugins[id];
  if (!found) throw new PluginError(`${id} isn't installed`);
  delete plugins[id];
  await writeInstalled(dir, plugins, rest);
  const root = typeof found.root === "string" ? found.root : undefined;
  const { name, marketplace } = splitId(id);
  // Only what the installer put in its cache.
  const owned = join(dir, "cache", marketplace ?? "", name);
  if (root !== undefined && isAbsolute(root) && inside(owned, root)) {
    await rm(owned, { recursive: true, force: true });
  }
}

/** Turns an installed plugin on or off. */
export async function setPluginEnabled(dir: string, id: string, enabled: boolean): Promise<void> {
  const { plugins, rest } = rawInstalled(dir);
  const found = plugins[id];
  if (!found) throw new PluginError(`${id} isn't installed`);
  found.enabled = enabled;
  await writeInstalled(dir, plugins, rest);
}

/**
 * The installed id the user means by `wanted`: `name@marketplace`, or just `name` when exactly one
 * installed plugin has it.
 */
export function resolveInstalled(dir: string, wanted: string): string {
  const ids = Object.keys(readInstalled(dir).plugins);
  if (ids.includes(wanted)) return wanted;
  const matches = ids.filter((id) => splitId(id).name === wanted);
  const [only] = matches;
  if (matches.length === 1 && only !== undefined) return only;
  if (matches.length > 1) throw new PluginError(`${wanted} is ambiguous: ${matches.join(", ")}`);
  throw new PluginError(`${wanted} isn't installed`);
}

/** The plugins of the added marketplaces, those whose name or description has `text`. */
export async function searchPlugins(
  dir: string,
  text: string,
): Promise<{ id: string; description: string }[]> {
  const query = text.trim().toLowerCase();
  const out: { id: string; description: string }[] = [];
  for (const record of Object.values(readMarketplaces(dir).marketplaces)) {
    let entries: MarketplaceEntry[];
    try {
      entries = await marketplaceEntries(record);
    } catch {
      continue;
    }
    for (const entry of entries) {
      let description = "";
      if (typeof entry.description === "string") description = entry.description;
      if (query === "" || `${entry.name} ${description}`.toLowerCase().includes(query)) {
        out.push({ id: `${entry.name}@${record.name}`, description });
      }
    }
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

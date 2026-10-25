import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ThinkingLevel } from "./loop/index.ts";
import { getEnvApiKey, getProviders } from "@mariozechner/pi-ai";
import type { SafeSearch } from "@neoworks/websearch";
import { accountProviders, resolveAccount, type AuthDeps, type ResolvedAccount } from "./auth.ts";
import type { SkillDirs } from "./skills.ts";
import { isRecord } from "./util.ts";

const DEFAULT_MODEL = "deepseek/deepseek-flash";
export const THINKING_LEVELS: readonly ThinkingLevel[] = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
];

export interface HarnessConfig {
  provider: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  /**
   * Whether the model takes images, overriding pi's registry (which lags behind new models).
   * Unset: what the registry says. Text-only models get a placeholder instead of each image.
   */
  images?: boolean;
  /** From NEO_API_KEY; used when the provider's own env var is not set. */
  apiKey?: string;
  /** Neo's own accounts file (NEO_AUTH), read per request and written by /login. */
  authPath: string;
  /** pi's auth file, read lazily as the last key source; also refreshed in place. */
  piAuthPath: string;
  /** For web_search and web_fetch. */
  web: WebConfig;
  /** Where skills are found besides the session's own; none without. */
  skills?: SkillDirs;
  /** Where the harness installs Claude Code plugins (plugins.ts); none without. */
  plugins?: string;
  /**
   * Base URLs that replace the models' own, by provider (`proxy` in config.json): requests of a
   * provider's models go to a proxy that forwards them. See proxy.ts.
   */
  proxy?: Record<string, string>;
  /** The user's agent definitions (agents.ts); none without. */
  agents?: string;
  /** Claude Code hooks (`hooks` in config.json), in Claude Code's settings shape; see hooks.ts. */
  hooks?: Record<string, unknown>;
}

/** The web tools' settings: the user's to choose, never the agent's. */
export interface WebConfig {
  /** How much adult content web_search may return (see @neoworks/websearch). */
  safeSearch: SafeSearch;
  /** A SearXNG instance to search alongside the other engines. */
  searxngUrl?: string;
}

const SAFE_SEARCH: readonly SafeSearch[] = ["strict", "moderate", "off"];

function parseSafeSearch(value: string, source: string): SafeSearch {
  const level = SAFE_SEARCH.find((candidate) => candidate === value);
  if (level) return level;
  throw new Error(`${source}: safe search must be one of ${SAFE_SEARCH.join(", ")}`);
}

/**
 * The `web` object of config.json (`safeSearch`, `searxng`), overridden by NEO_SAFE_SEARCH and
 * NEO_SEARXNG_URL.
 */
function parseWeb(
  file: Record<string, unknown> | undefined,
  path: string,
  env: NodeJS.ProcessEnv,
): WebConfig {
  const section = file?.web;
  if (section !== undefined && !isRecord(section))
    throw new Error(`${path}: web must be an object`);
  let safeSearch: SafeSearch = "moderate";
  if (typeof section?.safeSearch === "string") {
    safeSearch = parseSafeSearch(section.safeSearch, `${path} web.safeSearch`);
  }
  if (env.NEO_SAFE_SEARCH) safeSearch = parseSafeSearch(env.NEO_SAFE_SEARCH, "NEO_SAFE_SEARCH");
  const web: WebConfig = { safeSearch };
  if (env.NEO_SEARXNG_URL) web.searxngUrl = env.NEO_SEARXNG_URL;
  else if (typeof section?.searxng === "string" && section.searxng) {
    web.searxngUrl = section.searxng;
  }
  return web;
}

/** The `proxy` object of config.json: provider → http(s) URL. */
function parseProxy(value: unknown, path: string): Record<string, string> {
  if (!isRecord(value)) throw new Error(`${path}: proxy must be an object of provider: URL`);
  const proxy: Record<string, string> = {};
  for (const [provider, url] of Object.entries(value)) {
    if (!getProviders().some((known) => known === provider)) {
      throw new Error(`${path}: proxy: unknown provider "${provider}"`);
    }
    let parsed: URL | undefined;
    if (typeof url === "string") {
      try {
        parsed = new URL(url);
      } catch {
        // reported below
      }
    }
    if (typeof url !== "string" || parsed === undefined || !/^https?:$/.test(parsed.protocol)) {
      throw new Error(`${path}: proxy.${provider} must be an http(s) URL`);
    }
    proxy[provider] = url;
  }
  return proxy;
}

/** Parses a JSON file; undefined if it doesn't exist. Throws on invalid JSON. */
function readJson(path: string): unknown {
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`${path} is not valid JSON`);
  }
}

function parseThinkingLevel(value: string, source: string): ThinkingLevel {
  const level = THINKING_LEVELS.find((candidate) => candidate === value);
  if (level) return level;
  throw new Error(`${source}: thinking level must be one of ${THINKING_LEVELS.join(", ")}`);
}

function parseModel(value: string, source: string): { provider: string; modelId: string } {
  const slash = value.indexOf("/");
  if (slash <= 0 || slash === value.length - 1) {
    throw new Error(`${source}: model must look like "provider/id", got "${value}"`);
  }
  return { provider: value.slice(0, slash), modelId: value.slice(slash + 1) };
}

function parseImages(value: string, source: string): boolean {
  if (["1", "true", "yes"].includes(value)) return true;
  if (["0", "false", "no"].includes(value)) return false;
  throw new Error(`${source}: expected 1 or 0, got "${value}"`);
}

/** Neo's accounts file (NEO_AUTH, else in the data dir) and pi's, which is read only. */
export function authPaths(env: NodeJS.ProcessEnv = process.env): {
  authPath: string;
  piAuthPath: string;
} {
  const home = env.HOME || homedir();
  // Where the daemon keeps its data (daemon/src/paths.ts).
  const dataDir =
    env.NEO_DATA_DIR ||
    (env.XDG_DATA_HOME ? join(env.XDG_DATA_HOME, "neo") : join(home, ".local", "share", "neo"));
  return {
    authPath: env.NEO_AUTH || join(dataDir, "auth.json"),
    piAuthPath: env.NEO_PI_AUTH || join(home, ".pi", "agent", "auth.json"),
  };
}

/**
 * Reads `$XDG_CONFIG_HOME/neo/config.json` (optional; keys `model` = "provider/id",
 * `thinkingLevel`, `images`, `web`, `proxy` and `hooks`), overridden by NEO_MODEL, NEO_THINKING, NEO_IMAGES,
 * NEO_SAFE_SEARCH and NEO_SEARXNG_URL.
 */
export function loadHarnessConfig(env: NodeJS.ProcessEnv = process.env): HarnessConfig {
  const home = env.HOME || homedir();
  const configHome = env.XDG_CONFIG_HOME || join(home, ".config");
  const path = join(configHome, "neo", "config.json");
  const file = readJson(path);
  if (file !== undefined && !isRecord(file)) throw new Error(`${path} must contain an object`);

  let model = DEFAULT_MODEL;
  let modelSource = "default model";
  let thinking = "off";
  let thinkingSource = "default thinking level";
  if (file && typeof file.model === "string") {
    model = file.model;
    modelSource = path;
  }
  if (file && typeof file.thinkingLevel === "string") {
    thinking = file.thinkingLevel;
    thinkingSource = path;
  }
  if (env.NEO_MODEL) {
    model = env.NEO_MODEL;
    modelSource = "NEO_MODEL";
  }
  if (env.NEO_THINKING) {
    thinking = env.NEO_THINKING;
    thinkingSource = "NEO_THINKING";
  }

  const config: HarnessConfig = {
    ...parseModel(model, modelSource),
    thinkingLevel: parseThinkingLevel(thinking, thinkingSource),
    apiKey: env.NEO_API_KEY || undefined,
    ...authPaths(env),
    web: parseWeb(file, path, env),
    skills: {
      bundled: join(import.meta.dir, "..", "skills"),
      user: join(configHome, "neo", "skills"),
      plugins: join(configHome, "neo", "plugins"),
    },
    plugins: join(configHome, "neo", "plugins"),
    agents: join(configHome, "neo", "agents"),
  };
  if (env.NEO_IMAGES) config.images = parseImages(env.NEO_IMAGES, "NEO_IMAGES");
  else if (file && typeof file.images === "boolean") config.images = file.images;
  if (file?.proxy !== undefined) config.proxy = parseProxy(file.proxy, path);
  if (file?.hooks !== undefined) {
    if (!isRecord(file.hooks)) throw new Error(`${path}: hooks must be an object`);
    config.hooks = file.hooks;
  }
  return config;
}

/**
 * Provider env var (pi-ai) -> NEO_API_KEY -> neo's auth file -> pi's auth file. In the
 * files an API key counts as it is and an OAuth login as its access token, refreshed first (and
 * written back to the file it came from) when expired. Called per request, so a login added while
 * the daemon runs is picked up. `provider` defaults to the configured one; pass another when the
 * session uses a model of a different provider. Throws when a refresh fails.
 */
export async function resolveAuth(
  config: HarnessConfig,
  provider: string = config.provider,
  deps?: AuthDeps,
): Promise<ResolvedAccount | undefined> {
  const fromEnv = getEnvApiKey(provider);
  if (fromEnv) return { apiKey: fromEnv };
  if (config.apiKey) return { apiKey: config.apiKey };
  return (
    (await resolveAccount(config.authPath, provider, deps)) ??
    (await resolveAccount(config.piAuthPath, provider, deps))
  );
}

/** The key alone; see resolveAuth. */
export async function resolveApiKey(
  config: HarnessConfig,
  provider: string = config.provider,
  deps?: AuthDeps,
): Promise<string | undefined> {
  return (await resolveAuth(config, provider, deps))?.apiKey;
}

export function missingApiKeyMessage(
  config: HarnessConfig,
  provider: string = config.provider,
): string {
  const providerVar = `${provider.toUpperCase().replaceAll("-", "_")}_API_KEY`;
  return (
    `No API key or login for provider "${provider}". Set ${providerVar} or NEO_API_KEY ` +
    `in the daemon's environment, or add it to ${config.authPath} (pi's ${config.piAuthPath} ` +
    `works too).`
  );
}

/** Where a provider's credential comes from; the first that has one, in resolveAuth's order. */
export type CredentialSource = "env" | "config" | "neo" | "pi";

export interface ProviderCredential {
  provider: string;
  source: CredentialSource;
}

/**
 * Every pi-ai provider with a credential resolveAuth would find: its env var, NEO_API_KEY (for
 * the configured provider only), an entry in neo's auth file or in pi's. Reads, never
 * refreshes or writes. In pi-ai's provider order.
 */
export function providersWithCredential(
  config: HarnessConfig,
  deps: Pick<AuthDeps, "oauthProvider"> = {},
): ProviderCredential[] {
  const ours = new Set(accountProviders(config.authPath, deps.oauthProvider));
  const pis = new Set(accountProviders(config.piAuthPath, deps.oauthProvider));
  const found: ProviderCredential[] = [];
  for (const provider of getProviders()) {
    let source: CredentialSource | undefined;
    if (getEnvApiKey(provider)) source = "env";
    else if (config.apiKey && provider === config.provider) source = "config";
    else if (ours.has(provider)) source = "neo";
    else if (pis.has(provider)) source = "pi";
    if (source) found.push({ provider, source });
  }
  return found;
}

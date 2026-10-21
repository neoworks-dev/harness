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
  /** From SUITE_API_KEY; used when the provider's own env var is not set. */
  apiKey?: string;
  /** The suite's own accounts file (SUITE_AUTH), read per request and written by /login. */
  authPath: string;
  /** pi's auth file, read lazily as the last key source; also refreshed in place. */
  piAuthPath: string;
  /** For web_search and web_fetch. */
  web: WebConfig;
  /** Where skills are found besides the session's own; none without. */
  skills?: SkillDirs;
  /** Where the harness installs Claude Code plugins (plugins.ts); none without. */
  plugins?: string;
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
 * The `web` object of config.json (`safeSearch`, `searxng`), overridden by SUITE_SAFE_SEARCH and
 * SUITE_SEARXNG_URL.
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
  if (env.SUITE_SAFE_SEARCH)
    safeSearch = parseSafeSearch(env.SUITE_SAFE_SEARCH, "SUITE_SAFE_SEARCH");
  const web: WebConfig = { safeSearch };
  if (env.SUITE_SEARXNG_URL) web.searxngUrl = env.SUITE_SEARXNG_URL;
  else if (typeof section?.searxng === "string" && section.searxng) {
    web.searxngUrl = section.searxng;
  }
  return web;
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

/** The suite's accounts file (SUITE_AUTH, else in the data dir) and pi's, which is read only. */
export function authPaths(env: NodeJS.ProcessEnv = process.env): {
  authPath: string;
  piAuthPath: string;
} {
  const home = env.HOME || homedir();
  // Where the daemon keeps its data (daemon/src/paths.ts).
  const dataDir =
    env.SUITE_DATA_DIR ||
    (env.XDG_DATA_HOME ? join(env.XDG_DATA_HOME, "suite") : join(home, ".local", "share", "suite"));
  return {
    authPath: env.SUITE_AUTH || join(dataDir, "auth.json"),
    piAuthPath: env.SUITE_PI_AUTH || join(home, ".pi", "agent", "auth.json"),
  };
}

/**
 * Reads `$XDG_CONFIG_HOME/suite/config.json` (optional; keys `model` = "provider/id",
 * `thinkingLevel`, `images`, `web` and `hooks`), overridden by SUITE_MODEL, SUITE_THINKING, SUITE_IMAGES,
 * SUITE_SAFE_SEARCH and SUITE_SEARXNG_URL.
 */
export function loadHarnessConfig(env: NodeJS.ProcessEnv = process.env): HarnessConfig {
  const home = env.HOME || homedir();
  const configHome = env.XDG_CONFIG_HOME || join(home, ".config");
  const path = join(configHome, "suite", "config.json");
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
  if (env.SUITE_MODEL) {
    model = env.SUITE_MODEL;
    modelSource = "SUITE_MODEL";
  }
  if (env.SUITE_THINKING) {
    thinking = env.SUITE_THINKING;
    thinkingSource = "SUITE_THINKING";
  }

  const config: HarnessConfig = {
    ...parseModel(model, modelSource),
    thinkingLevel: parseThinkingLevel(thinking, thinkingSource),
    apiKey: env.SUITE_API_KEY || undefined,
    ...authPaths(env),
    web: parseWeb(file, path, env),
    skills: {
      bundled: join(import.meta.dir, "..", "skills"),
      user: join(configHome, "suite", "skills"),
    },
    plugins: join(configHome, "suite", "plugins"),
  };
  if (env.SUITE_IMAGES) config.images = parseImages(env.SUITE_IMAGES, "SUITE_IMAGES");
  else if (file && typeof file.images === "boolean") config.images = file.images;
  if (file?.hooks !== undefined) {
    if (!isRecord(file.hooks)) throw new Error(`${path}: hooks must be an object`);
    config.hooks = file.hooks;
  }
  return config;
}

/**
 * Provider env var (pi-ai) -> SUITE_API_KEY -> the suite's auth file -> pi's auth file. In the
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
    `No API key or login for provider "${provider}". Set ${providerVar} or SUITE_API_KEY ` +
    `in the daemon's environment, or add it to ${config.authPath} (pi's ${config.piAuthPath} ` +
    `works too).`
  );
}

/** Where a provider's credential comes from; the first that has one, in resolveAuth's order. */
export type CredentialSource = "env" | "config" | "suite" | "pi";

export interface ProviderCredential {
  provider: string;
  source: CredentialSource;
}

/**
 * Every pi-ai provider with a credential resolveAuth would find: its env var, SUITE_API_KEY (for
 * the configured provider only), an entry in the suite's auth file or in pi's. Reads, never
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
    else if (ours.has(provider)) source = "suite";
    else if (pis.has(provider)) source = "pi";
    if (source) found.push({ provider, source });
  }
  return found;
}

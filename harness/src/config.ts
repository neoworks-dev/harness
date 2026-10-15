import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ThinkingLevel } from "@mariozechner/pi-agent-core";
import { getEnvApiKey } from "@mariozechner/pi-ai";
import { isRecord } from "./util.ts";

const DEFAULT_MODEL = "deepseek/deepseek-v4-flash";
const THINKING_LEVELS: readonly ThinkingLevel[] = [
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
  /** From SUITE_API_KEY; used when the provider's own env var is not set. */
  apiKey?: string;
  /** pi's auth file, read lazily as the last key source. */
  piAuthPath: string;
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

/**
 * Reads `$XDG_CONFIG_HOME/suite/config.json` (optional; keys `model` = "provider/id" and
 * `thinkingLevel`), overridden by SUITE_MODEL and SUITE_THINKING.
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

  return {
    ...parseModel(model, modelSource),
    thinkingLevel: parseThinkingLevel(thinking, thinkingSource),
    apiKey: env.SUITE_API_KEY || undefined,
    piAuthPath: env.SUITE_PI_AUTH || join(home, ".pi", "agent", "auth.json"),
  };
}

/** API-key entries of pi's auth file only; OAuth entries are skipped. */
function readPiAuthKey(path: string, provider: string): string | undefined {
  let auth: unknown;
  try {
    auth = readJson(path);
  } catch {
    return undefined;
  }
  if (!isRecord(auth)) return undefined;
  const entry = auth[provider];
  if (!isRecord(entry) || typeof entry.key !== "string" || entry.key === "") return undefined;
  return entry.key;
}

/**
 * Provider env var (pi-ai) -> SUITE_API_KEY -> pi's auth file. Called per request so a key
 * added while the daemon runs is picked up.
 */
export function resolveApiKey(config: HarnessConfig): string | undefined {
  const fromEnv = getEnvApiKey(config.provider);
  if (fromEnv) return fromEnv;
  if (config.apiKey) return config.apiKey;
  return readPiAuthKey(config.piAuthPath, config.provider);
}

export function missingApiKeyMessage(config: HarnessConfig): string {
  const providerVar = `${config.provider.toUpperCase().replaceAll("-", "_")}_API_KEY`;
  return (
    `No API key for provider "${config.provider}". Set ${providerVar} or SUITE_API_KEY ` +
    `in the daemon's environment.`
  );
}

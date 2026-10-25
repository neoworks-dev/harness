// Which guests exist and how they start. Defaults: Claude Code and Codex through their ACP
// adapters (fetched by bunx on first use, pinned). `$XDG_CONFIG_HOME/neo/config.json` may set
// `guests: {claude: {title, command, permissions}, …}` (other names add guests); env overrides it:
// NEO_GUESTS=none (or a list like claude), NEO_GUEST_PERMISSIONS=ask|allow. Model, effort and
// mode are the user's choice per session (/model, /effort, /mode), not configured here.
// Their own subagents are off (Claude Code's Agent tool, Codex's multi_agent): delegate, which
// asks first and shows the work, is how agents hand work on here.
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { GuestConfig } from "./guests.ts";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Claude Code's tools that start subagents of its own (Task is the older name of Agent). */
export const CLAUDE_AGENT_TOOLS = ["Agent", "Task"];

/**
 * CODEX_CONFIG for the Codex adapter (config overrides it hands to Codex): the user's own, with
 * Codex's subagents off unless they turned them on there, and `extra` on top.
 */
export function codexConfig(
  extra: Record<string, unknown> = {},
  env: NodeJS.ProcessEnv = process.env,
): string {
  let config: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(env.CODEX_CONFIG ?? "{}");
    if (isRecord(parsed)) config = { ...parsed };
  } catch {
    // Not JSON: the adapter would refuse it, and so we replace it.
  }
  const features: Record<string, unknown> = { multi_agent: false, multi_agent_v2: false };
  if (isRecord(config.features)) Object.assign(features, config.features);
  return JSON.stringify({ ...config, features, ...extra });
}

function defaults(env: NodeJS.ProcessEnv): Record<string, GuestConfig> {
  // Prefer the user's own Claude Code over the one the adapter bundles.
  const claudeCli = Bun.which("claude");
  const claude: GuestConfig = {
    title: "Claude Code",
    command: ["bunx", "@agentclientprotocol/claude-agent-acp@0.91.0"],
    route: "claude",
    permissions: "ask",
  };
  if (claudeCli !== null) claude.env = { CLAUDE_CODE_EXECUTABLE: claudeCli };
  return {
    claude,
    codex: {
      title: "Codex",
      command: ["bunx", "@agentclientprotocol/codex-acp@2.2.2"],
      env: { CODEX_CONFIG: codexConfig({}, env) },
      route: "codex",
      permissions: "ask",
    },
  };
}

function readGuestsKey(env: NodeJS.ProcessEnv): Record<string, unknown> {
  const home = env.HOME || homedir();
  const path = join(env.XDG_CONFIG_HOME || join(home, ".config"), "neo", "config.json");
  let file: unknown;
  try {
    file = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return {}; // missing or invalid: the harness reports a broken config file
  }
  if (!isRecord(file) || !isRecord(file.guests)) return {};
  return file.guests;
}

function permissions(value: unknown, source: string): "ask" | "allow" {
  if (value === "ask" || value === "allow") return value;
  throw new Error(`${source}: permissions must be ask or allow`);
}

export function loadGuestConfigs(
  env: NodeJS.ProcessEnv = process.env,
): Record<string, GuestConfig> {
  const configs = defaults(env);
  for (const [name, value] of Object.entries(readGuestsKey(env))) {
    if (!isRecord(value)) continue;
    const config: GuestConfig = configs[name] ?? { title: name, command: [], permissions: "ask" };
    if (typeof value.title === "string") config.title = value.title;
    if (Array.isArray(value.command) && value.command.every((c) => typeof c === "string")) {
      config.command = value.command;
    }
    if (value.permissions !== undefined) {
      config.permissions = permissions(value.permissions, `config.json guests.${name}`);
    }
    if (config.command.length > 0) configs[name] = config;
  }
  if (env.NEO_GUEST_PERMISSIONS) {
    const value = permissions(env.NEO_GUEST_PERMISSIONS, "NEO_GUEST_PERMISSIONS");
    for (const config of Object.values(configs)) config.permissions = value;
  }
  if (env.NEO_GUESTS !== undefined) {
    const wanted = env.NEO_GUESTS.split(",").map((s) => s.trim()); // "none" matches none
    for (const name of Object.keys(configs)) if (!wanted.includes(name)) delete configs[name];
  }
  return configs;
}

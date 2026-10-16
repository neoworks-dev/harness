// Files the agents must not read or touch without the user saying so, in any approvals mode:
// environment files, private keys, credential stores and token-bearing config. Paths are
// resolved first (relative to the working directory, ~, symlinks when the file exists), so
// `cat ../.env` and a link to a key count. Shell commands are checked token by token.
import { lstatSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, isAbsolute, relative, resolve } from "node:path";

/** `.env.example` and friends are meant to be committed and read. */
const ENV_TEMPLATES = new Set([".env.example", ".env.sample", ".env.template"]);
const KEY_EXTENSIONS = [".pem", ".key", ".p12", ".pfx"];
const PRIVATE_KEY = /^id_(?:rsa|ed25519|ecdsa|dsa)(?:_.*)?$/;
/** Always sensitive, with everything below them. Relative to the home directory. */
const HOME_TREES = [".ssh", ".gnupg"];
/** Sensitive files below the home directory. */
const HOME_FILES = [
  ".aws/credentials",
  ".netrc",
  ".pi/agent/auth.json",
  ".config/gh/hosts.yml",
  ".docker/config.json",
  ".kube/config",
];
/** Config that may hold tokens, wherever it is. */
const TOKEN_CONFIGS = new Set([".npmrc", ".pypirc"]);

function sensitiveName(name: string): boolean {
  const lower = name.toLowerCase();
  if (lower === ".env") return true;
  if (lower.startsWith(".env.")) return !ENV_TEMPLATES.has(lower);
  if (KEY_EXTENSIONS.some((ext) => lower.endsWith(ext))) return true;
  if (PRIVATE_KEY.test(lower)) return !lower.endsWith(".pub");
  if (lower.includes("credentials") && lower.endsWith(".json")) return true;
  return TOKEN_CONFIGS.has(lower) || lower.startsWith("secrets.");
}

function inside(path: string, dir: string): boolean {
  const rel = relative(dir, path);
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
}

/** Whether an absolute path is sensitive as written. */
function sensitiveAbsolute(path: string, home: string): boolean {
  if (sensitiveName(basename(path))) return true;
  if (HOME_TREES.some((tree) => inside(path, resolve(home, tree)))) return true;
  return HOME_FILES.some((file) => path === resolve(home, file));
}

function expandHome(path: string, home: string): string {
  if (path === "~") return home;
  if (path.startsWith("~/")) return resolve(home, path.slice(2));
  return path;
}

function real(path: string): string | undefined {
  try {
    lstatSync(path);
    return realpathSync(path);
  } catch {
    return undefined;
  }
}

/**
 * The file's own path when it is sensitive (its real path when it is a link to one), else
 * undefined. `home` is for tests.
 */
export function sensitivePath(path: string, cwd: string, home = homedir()): string | undefined {
  const absolute = resolve(cwd, expandHome(path, home));
  if (sensitiveAbsolute(absolute, home)) return absolute;
  const target = real(absolute);
  if (target !== undefined && target !== absolute && sensitiveAbsolute(target, home)) return target;
  return undefined;
}

/** Splits a shell command into the words that could be paths. */
function words(command: string, home: string): string[] {
  return command
    .replaceAll("${HOME}", home)
    .replaceAll("$HOME", home)
    .split(/[\s;|&<>()`'"=,{}]+|\$\(/)
    .filter((word) => word !== "");
}

/** The sensitive paths a shell command mentions, each once. */
export function sensitiveInCommand(command: string, cwd: string, home = homedir()): string[] {
  const found = new Set<string>();
  for (const word of words(command, home)) {
    const path = sensitivePath(word, cwd, home);
    if (path !== undefined) found.add(path);
  }
  return [...found];
}

/** A path as the user would say it: relative to the working directory, else with ~. */
export function displayPath(path: string, cwd: string, home = homedir()): string {
  if (inside(path, cwd)) return relative(cwd, path) || ".";
  if (inside(path, home)) return `~/${relative(home, path)}`;
  return path;
}

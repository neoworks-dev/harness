// Accounts: API keys and OAuth logins in an auth file, in pi's format and with pi's lock, so the
// same file can be shared with a running pi (pi: packages/coding-agent/src/core/auth-storage.ts).
//
//   { "<provider>": { "type": "api_key", "key": "..." }
//                 | { "type": "oauth", "access": "...", "refresh": "...", "expires": <ms>, ... } }
//
// Refreshing an OAuth token rotates the refresh token, so two processes refreshing the same
// entry would log the user out. Every write, and a refresh, happens under pi's lock
// (proper-lockfile on the file itself, the lock being `<file>.lock`), and the entry is read again
// once the lock is held: whoever lost the race finds a fresh token and refreshes nothing.
import { chmod, mkdir, readFile, realpath, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Api, Model } from "@mariozechner/pi-ai";
import { getOAuthProvider, type OAuthCredentials } from "@mariozechner/pi-ai/oauth";
import lockfile from "proper-lockfile";
import { isRecord } from "./util.ts";

export interface ApiKeyAccount {
  type: "api_key";
  key: string;
}

export type OAuthAccount = { type: "oauth" } & OAuthCredentials;

export type Account = ApiKeyAccount | OAuthAccount;

/** Valid entries by provider id. */
export type Accounts = Record<string, Account>;

/** What the refresh needs from pi-ai's OAuth provider; tests pass their own. */
export interface OAuthRefresher {
  refreshToken(credentials: OAuthCredentials): Promise<OAuthCredentials>;
  getApiKey(credentials: OAuthCredentials): string;
  /** Provider quirks, e.g. Copilot's base URL; applied to the model of each request. */
  modifyModels?(models: Model<Api>[], credentials: OAuthCredentials): Model<Api>[];
}

export interface AuthDeps {
  /** Default: pi-ai's registry. */
  oauthProvider?: (id: string) => OAuthRefresher | undefined;
  /** Default: Date.now. */
  now?: () => number;
}

export interface ResolvedAccount {
  apiKey: string;
  /** For OAuth accounts whose provider changes the model (Copilot's base URL). */
  adjustModel?: (model: Model<Api>) => Model<Api>;
}

/** A refreshed token is wanted this long before it expires. */
const EXPIRY_MARGIN_MS = 30_000;

/** Same retries and stale time as pi, so we wait for each other the same way. */
const LOCK_OPTIONS = {
  realpath: false,
  stale: 30_000,
  retries: { retries: 10, factor: 2, minTimeout: 100, maxTimeout: 10_000, randomize: true },
};

type Raw = Record<string, unknown>;

function parseAccount(value: unknown): Account | undefined {
  if (!isRecord(value)) return undefined;
  if (value.type === "api_key") {
    return typeof value.key === "string" && value.key !== ""
      ? { type: "api_key", key: value.key }
      : undefined;
  }
  if (
    value.type === "oauth" &&
    typeof value.access === "string" &&
    typeof value.refresh === "string" &&
    typeof value.expires === "number"
  ) {
    return {
      ...value,
      type: "oauth",
      access: value.access,
      refresh: value.refresh,
      expires: value.expires,
    };
  }
  return undefined;
}

/** The file's entries as they are (unknown ones too, so a write keeps them). {} if it's missing. */
async function readRaw(path: string): Promise<Raw> {
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return {};
    throw error;
  }
  if (text.trim() === "") return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error(`${path} is not valid JSON`);
  }
  if (!isRecord(parsed)) throw new Error(`${path} must contain an object`);
  return parsed;
}

/** Valid entries of an auth file; {} if it doesn't exist. Throws when it can't be parsed. */
export async function readAccounts(path: string): Promise<Accounts> {
  const accounts: Accounts = {};
  for (const [provider, value] of Object.entries(await readRaw(path))) {
    const account = parseAccount(value);
    if (account) accounts[provider] = account;
  }
  return accounts;
}

/** Replaces the file's content (mode 0600) without readers ever seeing half of it. */
async function writeRaw(path: string, raw: Raw): Promise<void> {
  const target = await realpath(path); // a symlinked auth file (dotfiles) stays a symlink
  const temp = `${target}.${process.pid}.tmp`;
  await writeFile(temp, JSON.stringify(raw, null, 2), { mode: 0o600 });
  await chmod(temp, 0o600);
  await rename(temp, target);
}

/** Runs `fn` on the file's current content holding pi's lock; writes what it returns as `next`. */
async function withLock<T>(
  path: string,
  fn: (raw: Raw) => Promise<{ result: T; next?: Raw }>,
): Promise<T> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  try {
    await writeFile(path, "{}", { flag: "wx", mode: 0o600 });
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "EEXIST")) throw error;
  }
  let compromised: Error | undefined;
  const release = await lockfile.lock(path, {
    ...LOCK_OPTIONS,
    onCompromised: (error) => {
      compromised = error;
    },
  });
  try {
    const { result, next } = await fn(await readRaw(path));
    if (compromised) throw compromised;
    if (next) await writeRaw(path, next);
    return result;
  } finally {
    try {
      await release();
    } catch {
      // Lost lock (stale, or compromised): nothing left to release.
    }
  }
}

/** Stores the login for a provider (the file and its directory are created if missing). */
export async function setAccount(path: string, provider: string, account: Account): Promise<void> {
  await withLock(path, (raw) =>
    Promise.resolve({ result: undefined, next: { ...raw, [provider]: account } }),
  );
}

/** Removes a provider's login; false if there was none. */
export async function removeAccount(path: string, provider: string): Promise<boolean> {
  return withLock(path, (raw) => {
    if (!(provider in raw)) return Promise.resolve({ result: false });
    const { [provider]: _removed, ...rest } = raw;
    return Promise.resolve({ result: true, next: rest });
  });
}

function isFresh(account: OAuthAccount, now: number): boolean {
  return now < account.expires - EXPIRY_MARGIN_MS;
}

function oauthKey(account: OAuthAccount, refresher: OAuthRefresher): ResolvedAccount {
  return {
    apiKey: refresher.getApiKey(account),
    adjustModel: refresher.modifyModels
      ? (model) => refresher.modifyModels?.([model], account)[0] ?? model
      : undefined,
  };
}

async function refreshLocked(
  path: string,
  provider: string,
  refresher: OAuthRefresher,
  now: () => number,
): Promise<ResolvedAccount | undefined> {
  try {
    return await withLock(path, async (raw) => {
      // Someone may have refreshed (or logged out) while we waited for the lock.
      const current = parseAccount(raw[provider]);
      if (current?.type !== "oauth") return { result: undefined };
      if (isFresh(current, now())) return { result: oauthKey(current, refresher) };
      const refreshed: OAuthAccount = {
        ...current,
        ...(await refresher.refreshToken(current)),
        type: "oauth",
      };
      return {
        result: oauthKey(refreshed, refresher),
        next: { ...raw, [provider]: refreshed },
      };
    });
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`Could not refresh the "${provider}" login (${reason}). Log in again.`, {
      cause: error,
    });
  }
}

/**
 * The key for a provider from one auth file: an API key as stored, an OAuth token as the provider
 * turns it into a key, refreshed first (and written back to this file) when it has expired.
 * undefined when the file has nothing usable for the provider; throws when a refresh fails.
 */
export async function resolveAccount(
  path: string,
  provider: string,
  deps: AuthDeps = {},
): Promise<ResolvedAccount | undefined> {
  const now = deps.now ?? Date.now;
  let account: Account | undefined;
  try {
    account = (await readAccounts(path))[provider];
  } catch {
    return undefined; // missing or broken file: another source may still have the key
  }
  if (!account) return undefined;
  if (account.type === "api_key") return { apiKey: account.key };
  const refresher = (deps.oauthProvider ?? getOAuthProvider)(provider);
  if (!refresher) return undefined;
  if (isFresh(account, now())) return oauthKey(account, refresher);
  return refreshLocked(path, provider, refresher, now);
}

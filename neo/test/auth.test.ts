import { expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { chmod, readFile, stat, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Api, Model } from "@mariozechner/pi-ai";
import type { OAuthCredentials } from "@mariozechner/pi-ai/oauth";
import lockfile from "proper-lockfile";
import {
  readAccounts,
  removeAccount,
  resolveAccount,
  setAccount,
  type AuthDeps,
  type OAuthAccount,
  type OAuthRefresher,
} from "../src/auth.ts";
import { tempDir } from "./helpers.ts";

const NOW = 1_700_000_000_000;
const HOUR = 3_600_000;

const login = (
  access: string,
  expires: number,
  extra: Record<string, unknown> = {},
): OAuthAccount => ({
  type: "oauth" as const,
  access,
  refresh: `refresh-of-${access}`,
  expires,
  ...extra,
});

/** A provider that "refreshes" by minting access-N, and counts how often it was asked. */
function fakeProvider(options: { delayMs?: number; fail?: boolean; path?: string } = {}): {
  provider: OAuthRefresher;
  deps: AuthDeps;
  calls: OAuthCredentials[];
  lockDuringRefresh: () => boolean | undefined;
} {
  const calls: OAuthCredentials[] = [];
  let lockDuringRefresh: boolean | undefined;
  const provider: OAuthRefresher = {
    async refreshToken(credentials) {
      calls.push(credentials);
      if (options.path) lockDuringRefresh = existsSync(`${options.path}.lock`);
      if (options.delayMs) await new Promise((resolve) => setTimeout(resolve, options.delayMs));
      if (options.fail) throw new Error("invalid_grant");
      return {
        access: `access-${calls.length}`,
        refresh: `refresh-${calls.length}`,
        expires: NOW + HOUR,
      };
    },
    getApiKey: (credentials) => `key:${credentials.access}`,
  };
  const deps: AuthDeps = { oauthProvider: () => provider, now: () => NOW };
  return { provider, deps, calls, lockDuringRefresh: () => lockDuringRefresh };
}

async function authFile(content: unknown): Promise<string> {
  const path = join(await tempDir(), "auth.json");
  await writeFile(path, JSON.stringify(content), { mode: 0o600 });
  return path;
}

const mode = async (path: string): Promise<number> => (await stat(path)).mode & 0o777;
const readJson = async (path: string): Promise<unknown> => JSON.parse(await readFile(path, "utf8"));

test("setAccount creates the directory (0700) and the file (0600), readAccounts reads it back", async () => {
  const dir = join(await tempDir(), "data", "neo");
  const path = join(dir, "auth.json");
  await setAccount(path, "deepseek", { type: "api_key", key: "k1" });
  await setAccount(path, "openai-codex", login("a", NOW + HOUR));
  expect(await mode(dir)).toBe(0o700);
  expect(await mode(path)).toBe(0o600);
  expect(await readAccounts(path)).toEqual({
    deepseek: { type: "api_key", key: "k1" },
    "openai-codex": login("a", NOW + HOUR),
  });
});

test("writes keep other entries, even ones we don't understand, and tighten the mode", async () => {
  const path = await authFile({
    future: { type: "something-new", x: 1 },
    deepseek: { type: "api_key", key: "old" },
  });
  await chmod(path, 0o644);
  await setAccount(path, "deepseek", { type: "api_key", key: "new" });
  expect(await readJson(path)).toEqual({
    future: { type: "something-new", x: 1 },
    deepseek: { type: "api_key", key: "new" },
  });
  expect(await mode(path)).toBe(0o600);

  expect(await removeAccount(path, "deepseek")).toBe(true);
  expect(await removeAccount(path, "deepseek")).toBe(false);
  expect(await readJson(path)).toEqual({ future: { type: "something-new", x: 1 } });
});

test("a symlinked auth file stays a symlink", async () => {
  const dir = await tempDir();
  const real = join(dir, "real.json");
  await writeFile(real, "{}");
  const link = join(dir, "auth.json");
  await symlink(real, link);
  await setAccount(link, "deepseek", { type: "api_key", key: "k" });
  expect(await readJson(real)).toEqual({ deepseek: { type: "api_key", key: "k" } });
});

test("readAccounts: missing is empty, invalid entries are left out, broken JSON throws", async () => {
  const dir = await tempDir();
  expect(await readAccounts(join(dir, "nope.json"))).toEqual({});
  const path = await authFile({
    ok: { type: "api_key", key: "k" },
    noKey: { type: "api_key" },
    noType: { key: "k" },
    halfLogin: { type: "oauth", access: "a" },
  });
  expect(Object.keys(await readAccounts(path))).toEqual(["ok"]);
  await writeFile(path, "{oops");
  expect(readAccounts(path)).rejects.toThrow("not valid JSON");
});

test("API keys are returned as stored, OAuth tokens that haven't expired without a refresh", async () => {
  const { deps, calls } = fakeProvider();
  const path = await authFile({
    deepseek: { type: "api_key", key: "k1" },
    codex: login("fresh", NOW + HOUR),
  });
  expect(await resolveAccount(path, "deepseek", deps)).toEqual({ apiKey: "k1" });
  expect((await resolveAccount(path, "codex", deps))?.apiKey).toBe("key:fresh");
  expect(await resolveAccount(path, "missing", deps)).toBeUndefined();
  expect(calls).toHaveLength(0);
});

test("an expired login is refreshed under pi's lock and written back to its own file (0600)", async () => {
  {
    const path = await authFile({
      other: { type: "api_key", key: "untouched" },
      codex: login("stale", NOW - 1, { accountId: "acct" }),
    });
    await chmod(path, 0o644);
    const bystander = await authFile({ codex: login("not-ours", NOW - 1) });
    const { deps, calls, lockDuringRefresh } = fakeProvider({ path });

    const resolved = await resolveAccount(path, "codex", deps);
    expect(resolved?.apiKey).toBe("key:access-1");
    expect(calls).toHaveLength(1);
    expect(calls[0]?.refresh).toBe("refresh-of-stale");
    expect(lockDuringRefresh()).toBe(true);
    expect(existsSync(`${path}.lock`)).toBe(false);
    expect(await readJson(path)).toEqual({
      other: { type: "api_key", key: "untouched" },
      // Fields the refresh doesn't return are kept.
      codex: {
        type: "oauth",
        access: "access-1",
        refresh: "refresh-1",
        expires: NOW + HOUR,
        accountId: "acct",
      },
    });
    expect(await mode(path)).toBe(0o600);
    expect((await readAccounts(bystander)).codex).toMatchObject({ access: "not-ours" });
    await resolveAccount(path, "codex", deps);
    expect(calls).toHaveLength(1); // the second call finds the new token
  }
});

test("a token about to expire is refreshed too", async () => {
  const path = await authFile({ codex: login("soon", NOW + 5_000) });
  const { deps, calls } = fakeProvider();
  expect((await resolveAccount(path, "codex", deps))?.apiKey).toBe("key:access-1");
  expect(calls).toHaveLength(1);
});

test("concurrent resolutions refresh once", async () => {
  const path = await authFile({ codex: login("stale", NOW - 1) });
  const { deps, calls } = fakeProvider({ delayMs: 50 });
  const keys = await Promise.all(
    Array.from({ length: 4 }, () => resolveAccount(path, "codex", deps).then((r) => r?.apiKey)),
  );
  expect(keys).toEqual(Array(4).fill("key:access-1"));
  expect(calls).toHaveLength(1);
});

test("a refresh somebody else did while we waited for the lock is used, not repeated", async () => {
  const path = await authFile({ codex: login("stale", NOW - 1) });
  const { deps, calls } = fakeProvider();
  // Another process (pi) holds the lock, refreshes, writes, and lets go.
  const release = await lockfile.lock(path, { realpath: false });
  const resolving = resolveAccount(path, "codex", deps);
  await new Promise((resolve) => setTimeout(resolve, 30));
  await writeFile(path, JSON.stringify({ codex: login("by-pi", NOW + HOUR) }));
  await release();
  expect((await resolving)?.apiKey).toBe("key:by-pi");
  expect(calls).toHaveLength(0);
});

test("a failed refresh says to log in again and leaves the file alone", async () => {
  const content = { codex: login("stale", NOW - 1) };
  const path = await authFile(content);
  const { deps } = fakeProvider({ fail: true });
  const error = await resolveAccount(path, "codex", deps).catch((e: unknown) => e);
  expect(error).toBeInstanceOf(Error);
  expect((error as Error).message).toContain('"codex"');
  expect((error as Error).message).toContain("invalid_grant");
  expect((error as Error).message).toContain("Log in again");
  expect(await readJson(path)).toEqual(content);
  expect(existsSync(`${path}.lock`)).toBe(false);
});

test("a login of a provider pi-ai doesn't know is not usable", async () => {
  const path = await authFile({ mystery: login("t", NOW + HOUR) });
  expect(
    await resolveAccount(path, "mystery", { oauthProvider: () => undefined, now: () => NOW }),
  ).toBeUndefined();
});

test("providers that change the model (Copilot's base URL) get to", async () => {
  const path = await authFile({
    copilot: login("tok", NOW + HOUR, { enterpriseUrl: "ghe.example" }),
  });
  const { provider, deps } = fakeProvider();
  provider.modifyModels = (models, credentials) =>
    models.map((model) => ({ ...model, baseUrl: `https://${String(credentials.enterpriseUrl)}` }));
  const resolved = await resolveAccount(path, "copilot", deps);
  const model = { id: "m", baseUrl: "https://default" } as Model<Api>;
  expect(resolved?.adjustModel?.(model).baseUrl).toBe("https://ghe.example");
  expect(model.baseUrl).toBe("https://default");
  expect(
    (await resolveAccount(await authFile({ k: { type: "api_key", key: "x" } }), "k", deps))
      ?.adjustModel,
  ).toBeUndefined();
});

test("a file that can't be read gives nothing instead of throwing", async () => {
  const path = await authFile({});
  await writeFile(path, "{oops");
  expect(await resolveAccount(path, "codex", fakeProvider().deps)).toBeUndefined();
});

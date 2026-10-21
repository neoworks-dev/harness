import { afterEach, beforeEach, expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  loadHarnessConfig,
  missingApiKeyMessage,
  resolveApiKey,
  type HarnessConfig,
} from "../src/config.ts";
import { tempDir } from "./helpers.ts";

// pi-ai reads provider keys straight from process.env, so these tests have to clear them.
const PROVIDER_VAR = "DEEPSEEK_API_KEY";
let savedProviderKey: string | undefined;

beforeEach(() => {
  savedProviderKey = process.env[PROVIDER_VAR];
  delete process.env[PROVIDER_VAR];
});

afterEach(() => {
  if (savedProviderKey === undefined) delete process.env[PROVIDER_VAR];
  else process.env[PROVIDER_VAR] = savedProviderKey;
});

async function writeConfig(home: string, config: unknown): Promise<void> {
  await mkdir(join(home, ".config", "suite"), { recursive: true });
  await writeFile(join(home, ".config", "suite", "config.json"), JSON.stringify(config));
}

test("defaults without a config file", async () => {
  const home = await tempDir();
  expect(loadHarnessConfig({ HOME: home })).toEqual({
    provider: "deepseek",
    modelId: "deepseek-flash",
    thinkingLevel: "off",
    apiKey: undefined,
    authPath: join(home, ".local", "share", "suite", "auth.json"),
    piAuthPath: join(home, ".pi", "agent", "auth.json"),
    web: { safeSearch: "moderate" },
    skills: {
      bundled: join(import.meta.dir, "..", "skills"),
      user: join(home, ".config", "suite", "skills"),
      plugins: join(home, ".config", "suite", "plugins"),
    },
    plugins: join(home, ".config", "suite", "plugins"),
  });
});

test("the config file overrides the defaults; ids may contain slashes", async () => {
  const home = await tempDir();
  await writeConfig(home, { model: "openrouter/anthropic/claude-x", thinkingLevel: "high" });
  const config = loadHarnessConfig({ HOME: home });
  expect(config.provider).toBe("openrouter");
  expect(config.modelId).toBe("anthropic/claude-x");
  expect(config.thinkingLevel).toBe("high");
});

test("the web tools' settings: config.json's web object, then the environment", async () => {
  const home = await tempDir();
  await writeConfig(home, { web: { safeSearch: "strict", searxng: "http://127.0.0.1:8888" } });
  expect(loadHarnessConfig({ HOME: home }).web).toEqual({
    safeSearch: "strict",
    searxngUrl: "http://127.0.0.1:8888",
  });
  const fromEnv = loadHarnessConfig({
    HOME: home,
    SUITE_SAFE_SEARCH: "off",
    SUITE_SEARXNG_URL: "http://searx.lan",
  });
  expect(fromEnv.web).toEqual({ safeSearch: "off", searxngUrl: "http://searx.lan" });
  expect(() => loadHarnessConfig({ HOME: home, SUITE_SAFE_SEARCH: "none" })).toThrow(
    "SUITE_SAFE_SEARCH: safe search must be one of strict, moderate, off",
  );
});

test("XDG_CONFIG_HOME moves the config file", async () => {
  const xdg = await tempDir();
  await mkdir(join(xdg, "suite"));
  await writeFile(join(xdg, "suite", "config.json"), JSON.stringify({ model: "openai/gpt-x" }));
  expect(loadHarnessConfig({ HOME: "/nonexistent", XDG_CONFIG_HOME: xdg }).provider).toBe("openai");
});

test("environment variables override the config file", async () => {
  const home = await tempDir();
  await writeConfig(home, { model: "openai/gpt-x", thinkingLevel: "high" });
  const config = loadHarnessConfig({
    HOME: home,
    SUITE_MODEL: "anthropic/claude-y",
    SUITE_THINKING: "low",
    SUITE_PI_AUTH: "/some/auth.json",
    SUITE_AUTH: "/some/suite-auth.json",
  });
  expect(config).toMatchObject({
    provider: "anthropic",
    modelId: "claude-y",
    thinkingLevel: "low",
    piAuthPath: "/some/auth.json",
    authPath: "/some/suite-auth.json",
  });
});

test("the accounts file lives in the daemon's data dir", () => {
  const authPath = (env: Record<string, string>): string =>
    loadHarnessConfig({ HOME: "/h", ...env }).authPath;
  expect(authPath({})).toBe("/h/.local/share/suite/auth.json");
  expect(authPath({ XDG_DATA_HOME: "/x" })).toBe("/x/suite/auth.json");
  expect(authPath({ XDG_DATA_HOME: "/x", SUITE_DATA_DIR: "/d" })).toBe("/d/auth.json");
  expect(authPath({ SUITE_DATA_DIR: "/d", SUITE_AUTH: "/a.json" })).toBe("/a.json");
});

test("invalid values name where they came from", async () => {
  const home = await tempDir();
  expect(() => loadHarnessConfig({ HOME: home, SUITE_MODEL: "nomodel" })).toThrow("SUITE_MODEL");
  expect(() => loadHarnessConfig({ HOME: home, SUITE_THINKING: "max" })).toThrow("SUITE_THINKING");
  await writeConfig(home, { thinkingLevel: "bogus" });
  expect(() => loadHarnessConfig({ HOME: home })).toThrow("config.json");
});

const oauth = (extra: Record<string, unknown> = {}): Record<string, unknown> => ({
  type: "oauth",
  access: "access-1",
  refresh: "refresh-1",
  expires: Date.now() + 3_600_000,
  ...extra,
});

/** A config whose accounts file and pi's file hold the given content (undefined: no file). */
async function configWithAuth(
  files: { suite?: unknown; pi?: unknown },
  env: Record<string, string> = {},
): Promise<HarnessConfig> {
  const home = await tempDir();
  const suitePath = join(home, "suite-auth.json");
  const piPath = join(home, "pi-auth.json");
  if (files.suite !== undefined) await writeFile(suitePath, JSON.stringify(files.suite));
  if (files.pi !== undefined) await writeFile(piPath, JSON.stringify(files.pi));
  return loadHarnessConfig({ HOME: home, SUITE_AUTH: suitePath, SUITE_PI_AUTH: piPath, ...env });
}

const apiKey = (key: string): Record<string, unknown> => ({ deepseek: { type: "api_key", key } });

test("API key: provider env var, SUITE_API_KEY, the suite's auth file, then pi's", async () => {
  const files = { suite: apiKey("from-suite-auth"), pi: apiKey("from-pi-auth") };
  const config = await configWithAuth(files, { SUITE_API_KEY: "from-suite-env" });
  expect(await resolveApiKey(config)).toBe("from-suite-env");

  process.env[PROVIDER_VAR] = "from-provider-env";
  expect(await resolveApiKey(config)).toBe("from-provider-env");
  delete process.env[PROVIDER_VAR];

  expect(await resolveApiKey(await configWithAuth(files))).toBe("from-suite-auth");
  expect(await resolveApiKey(await configWithAuth({ pi: files.pi }))).toBe("from-pi-auth");
});

test("OAuth logins count in either file; one pi-ai can't use falls through to the next source", async () => {
  const codex = (token: string): Record<string, unknown> => ({
    "openai-codex": oauth({ access: token }),
  });
  const both = await configWithAuth({ suite: codex("suite-token"), pi: codex("pi-token") });
  expect(await resolveApiKey(both, "openai-codex")).toBe("suite-token");
  const piOnly = await configWithAuth({ pi: codex("pi-token") });
  expect(await resolveApiKey(piOnly, "openai-codex")).toBe("pi-token");

  // pi-ai has no OAuth provider for deepseek.
  const unknown = await configWithAuth({ suite: { deepseek: oauth() }, pi: apiKey("pi-key") });
  expect(await resolveApiKey(unknown)).toBe("pi-key");
});

test("malformed entries and files are skipped", async () => {
  const entries = { deepseek: { key: "no-type" }, other: { type: "api_key", key: "k" } };
  expect(
    await resolveApiKey(await configWithAuth({ suite: entries, pi: entries })),
  ).toBeUndefined();

  const home = await tempDir();
  const authPath = join(home, "auth.json");
  await writeFile(authPath, "not json");
  const config = loadHarnessConfig({ HOME: home, SUITE_AUTH: authPath, SUITE_PI_AUTH: authPath });
  expect(await resolveApiKey(config)).toBeUndefined();
  expect(await resolveApiKey(loadHarnessConfig({ HOME: home }))).toBeUndefined();
});

test("the auth files are read per call", async () => {
  const config = await configWithAuth({});
  expect(await resolveApiKey(config)).toBeUndefined();
  await writeFile(config.authPath, JSON.stringify(apiKey("late")));
  expect(await resolveApiKey(config)).toBe("late");
});

test("the missing key message names the env vars and the accounts file", async () => {
  const config = loadHarnessConfig({ HOME: await tempDir(), SUITE_AUTH: "/data/auth.json" });
  const message = missingApiKeyMessage(config);
  expect(message).toContain("DEEPSEEK_API_KEY");
  expect(message).toContain("SUITE_API_KEY");
  expect(message).toContain("/data/auth.json");
});

test("images: from the config file or SUITE_IMAGES, overriding pi's registry", async () => {
  const home = await tempDir();
  expect(loadHarnessConfig({ HOME: home }).images).toBeUndefined();
  await writeConfig(home, { model: "deepseek/deepseek-v4.1-flash", images: true });
  expect(loadHarnessConfig({ HOME: home }).images).toBe(true);
  expect(loadHarnessConfig({ HOME: home, SUITE_IMAGES: "0" }).images).toBe(false);
  expect(() => loadHarnessConfig({ HOME: home, SUITE_IMAGES: "maybe" })).toThrow("SUITE_IMAGES");

  const { resolveModel } = await import("../src/model.ts");
  expect(resolveModel("deepseek", "deepseek-v4.1-flash").input).toEqual(["text"]);
  expect(resolveModel("deepseek", "deepseek-flash").input).toEqual(["text", "image"]);
  expect(resolveModel("deepseek", "deepseek-flash", undefined, false).input).toEqual(["text"]);
  expect(resolveModel("deepseek", "deepseek-v4.1-flash", undefined, true).input).toEqual([
    "text",
    "image",
  ]);
});

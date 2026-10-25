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
  await mkdir(join(home, ".config", "neo"), { recursive: true });
  await writeFile(join(home, ".config", "neo", "config.json"), JSON.stringify(config));
}

test("defaults without a config file", async () => {
  const home = await tempDir();
  expect(loadHarnessConfig({ HOME: home })).toEqual({
    provider: "deepseek",
    modelId: "deepseek-flash",
    thinkingLevel: "off",
    apiKey: undefined,
    authPath: join(home, ".local", "share", "neo", "auth.json"),
    piAuthPath: join(home, ".pi", "agent", "auth.json"),
    web: { safeSearch: "moderate" },
    skills: {
      bundled: join(import.meta.dir, "..", "skills"),
      user: join(home, ".config", "neo", "skills"),
      plugins: join(home, ".config", "neo", "plugins"),
    },
    plugins: join(home, ".config", "neo", "plugins"),
    agents: join(home, ".config", "neo", "agents"),
  });
});

test("hooks come from config.json as they are; they must be an object", async () => {
  const home = await tempDir();
  const hooks = { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "x" }] }] };
  await writeConfig(home, { hooks });
  expect(loadHarnessConfig({ HOME: home }).hooks).toEqual(hooks);
  await writeConfig(home, { hooks: "nope" });
  expect(() => loadHarnessConfig({ HOME: home })).toThrow("hooks must be an object");
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
    NEO_SAFE_SEARCH: "off",
    NEO_SEARXNG_URL: "http://searx.lan",
  });
  expect(fromEnv.web).toEqual({ safeSearch: "off", searxngUrl: "http://searx.lan" });
  expect(() => loadHarnessConfig({ HOME: home, NEO_SAFE_SEARCH: "none" })).toThrow(
    "NEO_SAFE_SEARCH: safe search must be one of strict, moderate, off",
  );
});

test("XDG_CONFIG_HOME moves the config file", async () => {
  const xdg = await tempDir();
  await mkdir(join(xdg, "neo"));
  await writeFile(join(xdg, "neo", "config.json"), JSON.stringify({ model: "openai/gpt-x" }));
  expect(loadHarnessConfig({ HOME: "/nonexistent", XDG_CONFIG_HOME: xdg }).provider).toBe("openai");
});

test("environment variables override the config file", async () => {
  const home = await tempDir();
  await writeConfig(home, { model: "openai/gpt-x", thinkingLevel: "high" });
  const config = loadHarnessConfig({
    HOME: home,
    NEO_MODEL: "anthropic/claude-y",
    NEO_THINKING: "low",
    NEO_PI_AUTH: "/some/auth.json",
    NEO_AUTH: "/some/neo-auth.json",
  });
  expect(config).toMatchObject({
    provider: "anthropic",
    modelId: "claude-y",
    thinkingLevel: "low",
    piAuthPath: "/some/auth.json",
    authPath: "/some/neo-auth.json",
  });
});

test("the accounts file lives in the daemon's data dir", () => {
  const authPath = (env: Record<string, string>): string =>
    loadHarnessConfig({ HOME: "/h", ...env }).authPath;
  expect(authPath({})).toBe("/h/.local/share/neo/auth.json");
  expect(authPath({ XDG_DATA_HOME: "/x" })).toBe("/x/neo/auth.json");
  expect(authPath({ XDG_DATA_HOME: "/x", NEO_DATA_DIR: "/d" })).toBe("/d/auth.json");
  expect(authPath({ NEO_DATA_DIR: "/d", NEO_AUTH: "/a.json" })).toBe("/a.json");
});

test("invalid values name where they came from", async () => {
  const home = await tempDir();
  expect(() => loadHarnessConfig({ HOME: home, NEO_MODEL: "nomodel" })).toThrow("NEO_MODEL");
  expect(() => loadHarnessConfig({ HOME: home, NEO_THINKING: "max" })).toThrow("NEO_THINKING");
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
  files: { neo?: unknown; pi?: unknown },
  env: Record<string, string> = {},
): Promise<HarnessConfig> {
  const home = await tempDir();
  const neoPath = join(home, "neo-auth.json");
  const piPath = join(home, "pi-auth.json");
  if (files.neo !== undefined) await writeFile(neoPath, JSON.stringify(files.neo));
  if (files.pi !== undefined) await writeFile(piPath, JSON.stringify(files.pi));
  return loadHarnessConfig({ HOME: home, NEO_AUTH: neoPath, NEO_PI_AUTH: piPath, ...env });
}

const apiKey = (key: string): Record<string, unknown> => ({ deepseek: { type: "api_key", key } });

test("API key: provider env var, NEO_API_KEY, neo's auth file, then pi's", async () => {
  const files = { neo: apiKey("from-neo-auth"), pi: apiKey("from-pi-auth") };
  const config = await configWithAuth(files, { NEO_API_KEY: "from-neo-env" });
  expect(await resolveApiKey(config)).toBe("from-neo-env");

  process.env[PROVIDER_VAR] = "from-provider-env";
  expect(await resolveApiKey(config)).toBe("from-provider-env");
  delete process.env[PROVIDER_VAR];

  expect(await resolveApiKey(await configWithAuth(files))).toBe("from-neo-auth");
  expect(await resolveApiKey(await configWithAuth({ pi: files.pi }))).toBe("from-pi-auth");
});

test("OAuth logins count in either file; one pi-ai can't use falls through to the next source", async () => {
  const codex = (token: string): Record<string, unknown> => ({
    "openai-codex": oauth({ access: token }),
  });
  const both = await configWithAuth({ neo: codex("neo-token"), pi: codex("pi-token") });
  expect(await resolveApiKey(both, "openai-codex")).toBe("neo-token");
  const piOnly = await configWithAuth({ pi: codex("pi-token") });
  expect(await resolveApiKey(piOnly, "openai-codex")).toBe("pi-token");

  // pi-ai has no OAuth provider for deepseek.
  const unknown = await configWithAuth({ neo: { deepseek: oauth() }, pi: apiKey("pi-key") });
  expect(await resolveApiKey(unknown)).toBe("pi-key");
});

test("malformed entries and files are skipped", async () => {
  const entries = { deepseek: { key: "no-type" }, other: { type: "api_key", key: "k" } };
  expect(await resolveApiKey(await configWithAuth({ neo: entries, pi: entries }))).toBeUndefined();

  const home = await tempDir();
  const authPath = join(home, "auth.json");
  await writeFile(authPath, "not json");
  const config = loadHarnessConfig({ HOME: home, NEO_AUTH: authPath, NEO_PI_AUTH: authPath });
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
  const config = loadHarnessConfig({ HOME: await tempDir(), NEO_AUTH: "/data/auth.json" });
  const message = missingApiKeyMessage(config);
  expect(message).toContain("DEEPSEEK_API_KEY");
  expect(message).toContain("NEO_API_KEY");
  expect(message).toContain("/data/auth.json");
});

test("images: from the config file or NEO_IMAGES, overriding pi's registry", async () => {
  const home = await tempDir();
  expect(loadHarnessConfig({ HOME: home }).images).toBeUndefined();
  await writeConfig(home, { model: "deepseek/deepseek-v4.1-flash", images: true });
  expect(loadHarnessConfig({ HOME: home }).images).toBe(true);
  expect(loadHarnessConfig({ HOME: home, NEO_IMAGES: "0" }).images).toBe(false);
  expect(() => loadHarnessConfig({ HOME: home, NEO_IMAGES: "maybe" })).toThrow("NEO_IMAGES");

  const { resolveModel } = await import("../src/model.ts");
  expect(resolveModel("deepseek", "deepseek-v4.1-flash").input).toEqual(["text"]);
  expect(resolveModel("deepseek", "deepseek-flash").input).toEqual(["text", "image"]);
  expect(resolveModel("deepseek", "deepseek-flash", undefined, false).input).toEqual(["text"]);
  expect(resolveModel("deepseek", "deepseek-v4.1-flash", undefined, true).input).toEqual([
    "text",
    "image",
  ]);
});

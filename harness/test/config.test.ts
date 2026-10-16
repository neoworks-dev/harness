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
    piAuthPath: join(home, ".pi", "agent", "auth.json"),
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
  });
  expect(config).toMatchObject({
    provider: "anthropic",
    modelId: "claude-y",
    thinkingLevel: "low",
    piAuthPath: "/some/auth.json",
  });
});

test("invalid values name where they came from", async () => {
  const home = await tempDir();
  expect(() => loadHarnessConfig({ HOME: home, SUITE_MODEL: "nomodel" })).toThrow("SUITE_MODEL");
  expect(() => loadHarnessConfig({ HOME: home, SUITE_THINKING: "max" })).toThrow("SUITE_THINKING");
  await writeConfig(home, { thinkingLevel: "bogus" });
  expect(() => loadHarnessConfig({ HOME: home })).toThrow("config.json");
});

async function configWithAuth(
  auth: unknown,
  env: Record<string, string> = {},
): Promise<HarnessConfig> {
  const home = await tempDir();
  const authPath = join(home, "auth.json");
  await writeFile(authPath, JSON.stringify(auth));
  return loadHarnessConfig({ HOME: home, SUITE_PI_AUTH: authPath, ...env });
}

test("API key: provider env var, then SUITE_API_KEY, then pi's auth file", async () => {
  const auth = { deepseek: { type: "api_key", key: "from-auth" } };
  const config = await configWithAuth(auth, { SUITE_API_KEY: "from-suite" });
  expect(resolveApiKey(config)).toBe("from-suite");

  process.env[PROVIDER_VAR] = "from-provider-env";
  expect(resolveApiKey(config)).toBe("from-provider-env");
  delete process.env[PROVIDER_VAR];

  expect(resolveApiKey(await configWithAuth(auth))).toBe("from-auth");
});

test("OAuth entries and malformed auth files are skipped", async () => {
  const oauth = { deepseek: { type: "oauth", access: "a", refresh: "r" } };
  expect(resolveApiKey(await configWithAuth(oauth))).toBeUndefined();
  expect(resolveApiKey(await configWithAuth({ other: { key: "k" } }))).toBeUndefined();

  const home = await tempDir();
  const authPath = join(home, "auth.json");
  await writeFile(authPath, "not json");
  expect(resolveApiKey(loadHarnessConfig({ HOME: home, SUITE_PI_AUTH: authPath }))).toBeUndefined();
  expect(resolveApiKey(loadHarnessConfig({ HOME: home }))).toBeUndefined();
});

test("the auth file is read per call", async () => {
  const home = await tempDir();
  const authPath = join(home, "auth.json");
  const config = loadHarnessConfig({ HOME: home, SUITE_PI_AUTH: authPath });
  expect(resolveApiKey(config)).toBeUndefined();
  await writeFile(authPath, JSON.stringify({ deepseek: { key: "late" } }));
  expect(resolveApiKey(config)).toBe("late");
});

test("the missing key message names the env vars to set", async () => {
  const message = missingApiKeyMessage(loadHarnessConfig({ HOME: await tempDir() }));
  expect(message).toContain("DEEPSEEK_API_KEY");
  expect(message).toContain("SUITE_API_KEY");
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

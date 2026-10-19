// Which providers have a credential, and /model listing the models of all of them. Everything runs
// on temp auth files and a cleared environment: never the real ~/.pi or ~/.local/share/suite.
import { expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { OAuthRefresher } from "../src/auth.ts";
import { providersWithCredential, type HarnessConfig } from "../src/config.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import type { AgentSession } from "../src/agent.ts";
import { fakeModel } from "./fake-model.ts";
import { errorMessage, tempDir } from "./helpers.ts";

const FAR = Date.now() + 3_600_000;
const api = (key: string): unknown => ({ type: "api_key", key });
const oauth = (expires: number): unknown => ({
  type: "oauth",
  access: "a",
  refresh: "r",
  expires,
});
/** Only these have a refresher, as if pi-ai's registry held just them. */
const KNOWN_LOGINS = new Set(["openai-codex", "github-copilot"]);
const oauthProvider = (id: string): OAuthRefresher | undefined =>
  KNOWN_LOGINS.has(id) ? ({} as OAuthRefresher) : undefined;

/**
 * Runs `body` with process.env holding only `vars` (pi-ai reads provider keys from it). One dummy
 * entry stays so that bun does not fall back to reading /proc/self/environ.
 */
async function withEnv<T>(vars: Record<string, string>, body: () => Promise<T>): Promise<T> {
  const saved = { ...process.env };
  for (const name of Object.keys(process.env)) delete process.env[name];
  Object.assign(process.env, { HARNESS_TEST: "1", ...vars });
  try {
    return await body();
  } finally {
    for (const name of Object.keys(process.env)) delete process.env[name];
    Object.assign(process.env, saved);
  }
}

async function configWith(
  suite: Record<string, unknown> | undefined,
  pi: Record<string, unknown> | undefined,
  extra: Partial<HarnessConfig> = {},
): Promise<HarnessConfig> {
  const dir = await tempDir();
  const authPath = join(dir, "suite", "auth.json");
  const piAuthPath = join(dir, "pi", "auth.json");
  await mkdir(join(dir, "suite"));
  await mkdir(join(dir, "pi"));
  if (suite) await writeFile(authPath, JSON.stringify(suite));
  if (pi) await writeFile(piAuthPath, JSON.stringify(pi));
  return {
    provider: "deepseek",
    modelId: "deepseek-v4-flash",
    thinkingLevel: "off",
    authPath,
    piAuthPath,
    ...extra,
  };
}

test("providers with a credential: env var, the config key for its provider, our file, pi's file", async () => {
  const config = await configWith(
    {
      openai: api("sk-ours"),
      "openai-codex": oauth(1), // expired, still counts: it refreshes on use
      anthropic: oauth(FAR), // pi-ai has no login flow for it here
      "no-such-provider": api("x"),
      groq: { type: "api_key", key: "" }, // not a usable entry
    },
    { openai: api("sk-pi"), xai: api("xai-pi"), "github-copilot": oauth(FAR) },
    { apiKey: "from-suite-api-key" },
  );
  const found = await withEnv({ OPENROUTER_API_KEY: "or-key", MISTRAL_API_KEY: "m" }, () =>
    Promise.resolve(providersWithCredential(config, { oauthProvider })),
  );
  expect(found).toEqual([
    { provider: "deepseek", source: "config" },
    { provider: "github-copilot", source: "pi" },
    { provider: "mistral", source: "env" },
    { provider: "openai", source: "suite" },
    { provider: "openai-codex", source: "suite" },
    { provider: "openrouter", source: "env" },
    { provider: "xai", source: "pi" },
  ]);
});

test("SUITE_API_KEY counts for the configured provider only, and missing files for nobody", async () => {
  const config = await configWith(undefined, undefined, { apiKey: "k", provider: "openai" });
  const found = await withEnv({}, () => Promise.resolve(providersWithCredential(config)));
  expect(found).toEqual([{ provider: "openai", source: "config" }]);
  const bare = await configWith(undefined, undefined);
  expect(await withEnv({}, () => Promise.resolve(providersWithCredential(bare)))).toEqual([]);
});

test("a broken auth file is skipped, not an error", async () => {
  const config = await configWith(undefined, undefined);
  await writeFile(config.authPath, "{ not json");
  await writeFile(config.piAuthPath, "[1, 2]");
  expect(await withEnv({}, () => Promise.resolve(providersWithCredential(config)))).toEqual([]);
});

async function sessionFor(config: HarnessConfig, model?: string): Promise<AgentSession> {
  const agent = createNeoAgent({ config, streamFn: fakeModel([]).streamFn });
  return agent.newSession({
    sessionId: "s1",
    cwd: await tempDir(),
    mcpServers: [],
    settings: model === undefined ? undefined : { model },
  });
}

const modelOption = (
  session: AgentSession,
): { current?: string; values: { value: string; name: string }[] } => {
  const option = session.configOptions?.().find((o) => o.category === "model");
  if (!option) throw new Error("no model option");
  return option;
};

test("/model lists the models of every provider that has a credential, the current one first", async () => {
  const config = await configWith(
    { openrouter: api("or-key") },
    { openai: api("sk-pi") },
    { apiKey: "from-suite-api-key" },
  );
  await withEnv({}, async () => {
    const option = modelOption(await sessionFor(config));
    const values = option.values.map((v) => v.value);
    expect(values[0]).toBe("deepseek/deepseek-v4-flash");
    expect(option.current).toBe("deepseek/deepseek-v4-flash");
    expect(new Set(values).size).toBe(values.length);
    const providers = new Set(values.map((v) => v.slice(0, v.indexOf("/"))));
    expect([...providers].sort((a, b) => a.localeCompare(b))).toEqual([
      "deepseek",
      "openai",
      "openrouter",
    ]);
    // The current provider's models come before the others'.
    const lastDeepseek = values.findLastIndex((v) => v.startsWith("deepseek/"));
    const firstOther = values.findIndex((v) => !v.startsWith("deepseek/"));
    expect(lastDeepseek).toBeLessThan(firstOther);
    // An id with a slash of its own keeps its provider in front.
    expect(values).toContain("openrouter/anthropic/claude-haiku-4.5");
    expect(values).toContain("openai/gpt-4o");
    // With several providers a name says whose it is.
    const haiku = option.values.find((v) => v.value === "openrouter/anthropic/claude-haiku-4.5");
    expect(haiku?.name).toEndWith("· openrouter");
  });
});

test("a provider without a credential is not listed, and one provider keeps plain names", async () => {
  const config = await configWith(undefined, undefined, { apiKey: "k" });
  await withEnv({}, async () => {
    const option = modelOption(await sessionFor(config));
    expect(new Set(option.values.map((v) => v.value.split("/")[0]))).toEqual(new Set(["deepseek"]));
    expect(option.values.some((v) => v.name.includes("·"))).toBe(false);
  });
});

test("the current model stays first when it belongs to another provider or is unknown", async () => {
  const config = await configWith({ openrouter: api("or-key") }, undefined, { apiKey: "k" });
  await withEnv({}, async () => {
    const other = modelOption(await sessionFor(config, "openrouter/anthropic/claude-haiku-4.5"));
    expect(other.current).toBe("openrouter/anthropic/claude-haiku-4.5");
    expect(other.values[0]?.value).toBe("openrouter/anthropic/claude-haiku-4.5");
    expect(other.values.filter((v) => v.value === other.current)).toHaveLength(1);
    // The provider of the current model leads even though it is not the configured one.
    expect(other.values[1]?.value.startsWith("openrouter/")).toBe(true);

    const unknown = modelOption(await sessionFor(config, "deepseek/deepseek-v9"));
    expect(unknown.values[0]?.value).toBe("deepseek/deepseek-v9");
  });
});

test("a listed value from another provider can be set, also when its id contains a slash", async () => {
  const config = await configWith(
    { openrouter: api("or-key") },
    { openai: api("sk-pi") },
    {
      apiKey: "k",
    },
  );
  await withEnv({}, async () => {
    const session = await sessionFor(config);
    for (const value of [
      "openrouter/anthropic/claude-haiku-4.5",
      "openai/gpt-4o",
      "deepseek/deepseek-v4-pro",
    ]) {
      await session.setConfigOption?.("model", value);
      expect(modelOption(session).current).toBe(value);
    }
    // From an openrouter-configured neo too, where bare openrouter ids contain a slash.
    const onRouter = await sessionFor({
      ...config,
      provider: "openrouter",
      modelId: "openai/gpt-4o",
    });
    expect(modelOption(onRouter).current).toBe("openrouter/openai/gpt-4o");
    await onRouter.setConfigOption?.("model", "openrouter/anthropic/claude-haiku-4.5");
    expect(modelOption(onRouter).current).toBe("openrouter/anthropic/claude-haiku-4.5");
    await onRouter.setConfigOption?.("model", "anthropic/claude-3.5-haiku"); // bare openrouter id
    expect(modelOption(onRouter).current).toBe("openrouter/anthropic/claude-3.5-haiku");
    await onRouter.setConfigOption?.("model", "openai/gpt-4o"); // explicit provider wins
    expect(modelOption(onRouter).current).toBe("openai/gpt-4o");
    const failure = await errorMessage(
      onRouter.setConfigOption?.("model", "nope/x") ?? Promise.resolve(),
    );
    expect(failure).toContain("Unknown model provider");
  });
});

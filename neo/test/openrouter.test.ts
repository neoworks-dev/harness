// OpenRouter's live model list, the "@host" suffix on a model, and the host choice that follows
// /model. The network is always a fake fetch serving trimmed real responses (test/fixtures).
import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { getModels } from "@earendil-works/pi-ai/compat";
import type { AgentSession, ConfigOption } from "../src/agent.ts";
import type { StreamFn } from "../src/loop/index.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import {
  endpointLabel,
  mergeLiveModels,
  OpenRouterCatalog,
  parseEndpoints,
  type FetchFn,
} from "../src/openrouter.ts";
import { fakeModel, reply, TEST_CONFIG } from "./fake-model.ts";
import { errorMessage, tempDir } from "./helpers.ts";

const fixture = async (name: string): Promise<unknown> =>
  JSON.parse(await readFile(join(import.meta.dir, "fixtures", name), "utf8"));
const MODELS = await fixture("openrouter-models.json");
const ENDPOINTS = await fixture("openrouter-endpoints.json");

const NEW = "openrouter/deepseek/deepseek-v9-flash-1017";
const NEW_NAME = "DeepSeek: DeepSeek V9 Flash 1017";
const CONFIG = { ...TEST_CONFIG, provider: "openrouter", modelId: "anthropic/claude-haiku-4.5" };

interface FakeNetwork {
  fetch: FetchFn;
  urls: string[];
}

/** Serves the fixtures; `fail` makes every request fail. */
function network(options: { fail?: boolean; endpoints?: unknown } = {}): FakeNetwork {
  const urls: string[] = [];
  const fetch: FetchFn = (url) => {
    urls.push(url);
    if (options.fail) return Promise.reject(new Error("offline"));
    if (url.endsWith("/models")) return Promise.resolve(Response.json(MODELS));
    if (url.endsWith("/endpoints"))
      return Promise.resolve(Response.json(options.endpoints ?? ENDPOINTS));
    return Promise.resolve(new Response("not found", { status: 404 }));
  };
  return { fetch, urls };
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 200; i++) {
    if (check()) return;
    await Bun.sleep(5);
  }
  throw new Error(`timed out waiting for ${what}`);
}

const modelOption = (session: AgentSession): ConfigOption => {
  const option = session.configOptions?.().find((o) => o.category === "model");
  if (!option) throw new Error("no model option");
  return option;
};

const listed = (session: AgentSession, value: string): boolean =>
  modelOption(session).values.some((v) => v.value === value);

async function openRouterSession(
  net: FakeNetwork,
  settings?: { model: string },
  streamFn: StreamFn = fakeModel([]).streamFn,
): Promise<AgentSession> {
  const agent = createNeoAgent({ config: CONFIG, streamFn, fetch: net.fetch });
  return agent.newSession({ sessionId: "s1", cwd: await tempDir(), mcpServers: [], settings });
}

// The live list

test("the live list adds models pi-ai lacks, keeps pi-ai's entry where it has one", () => {
  const bundled = getModels("openrouter");
  const merged = mergeLiveModels(MODELS, bundled);
  expect(merged.map((m) => m.id)).toEqual([
    "deepseek/deepseek-v9-flash-1017",
    "anthropic/claude-haiku-4.5",
    "openai/gpt-4o-mini",
    "openrouter/auto",
  ]);
  const fresh = merged[0];
  expect(fresh).toMatchObject({
    provider: "openrouter",
    api: "openai-completions",
    baseUrl: "https://openrouter.ai/api/v1",
    name: NEW_NAME,
    contextWindow: 1048576,
    maxTokens: 393216,
    reasoning: true,
    input: ["text", "image"],
    cost: { input: 0.15, output: 0.6, cacheRead: 0.003, cacheWrite: 0 },
  });
  // The same object pi-ai has, not a rebuilt one.
  expect(merged[1]).toBe(bundled.find((m) => m.id === "anthropic/claude-haiku-4.5"));
  // A price of -1 (the auto router; pi-ai has it too) is no price.
  const auto = merged.find((m) => m.id === "openrouter/auto");
  expect(auto?.cost).toEqual({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 });
});

test("a model that can't answer in text, and junk entries, are left out", () => {
  const body = {
    data: [
      { id: "a/image-gen", context_length: 1000, architecture: { output_modalities: ["image"] } },
      { id: "a/no-context", architecture: { output_modalities: ["text"] } },
      "junk",
      { context_length: 1000 },
      { id: "a/ok", name: "Ok", context_length: 4000, pricing: { prompt: "0.000002" } },
      { id: "a/ok", context_length: 1 },
    ],
  };
  const merged = mergeLiveModels(body, getModels("openrouter"));
  expect(merged.map((m) => m.id)).toEqual(["a/ok"]);
  expect(merged[0]).toMatchObject({ input: ["text"], contextWindow: 4000, cost: { input: 2 } });
  expect(() => mergeLiveModels({ data: [] }, getModels("openrouter"))).toThrow("empty");
  expect(() => mergeLiveModels({}, getModels("openrouter"))).toThrow("no data");
});

test("the catalog fetches once, serves the list from memory and refreshes after an hour", async () => {
  const net = network();
  let now = 1_000_000;
  const catalog = new OpenRouterCatalog({ fetch: net.fetch, now: () => now });
  expect(catalog.cached()).toBeUndefined(); // starts the fetch, does not wait for it
  await catalog.refresh();
  expect(catalog.cached()?.[0]?.id).toBe("deepseek/deepseek-v9-flash-1017");
  catalog.cached();
  now += 59 * 60_000;
  catalog.cached();
  expect(net.urls).toEqual(["https://openrouter.ai/api/v1/models"]);
  now += 2 * 60_000;
  catalog.cached(); // stale: the old list is still served while the new one loads
  expect(catalog.cached()).toBeDefined();
  await catalog.refresh();
  expect(net.urls).toHaveLength(2);
});

test("a failed fetch leaves no list, is not retried at once and keeps an older list", async () => {
  const warnings: string[] = [];
  const logger = { warn: (m: string) => warnings.push(m), error: () => {} };
  const net = network({ fail: true });
  let now = 1_000_000;
  const catalog = new OpenRouterCatalog({ fetch: net.fetch, now: () => now, logger });
  catalog.cached();
  await catalog.refresh();
  expect(catalog.cached()).toBeUndefined();
  expect(net.urls).toHaveLength(1);
  expect(warnings[0]).toContain("offline");
  now += 61_000;
  catalog.cached();
  await catalog.refresh();
  expect(net.urls).toHaveLength(2);

  let up = true;
  const flaky = new OpenRouterCatalog({
    fetch: (url, init) => (up ? network().fetch(url, init) : Promise.reject(new Error("down"))),
    now: () => now,
  });
  await flaky.refresh();
  up = false;
  now += 2 * 3_600_000;
  await flaky.refresh();
  expect(flaky.cached()).toHaveLength(4);
});

test("a bad status is a failure", async () => {
  const catalog = new OpenRouterCatalog({
    fetch: () => Promise.resolve(new Response("", { status: 503 })),
  });
  expect(await errorMessage(catalog.endpoints("a/b"))).toContain("503");
});

// /model in neo

test("/model lists the live models once they arrived, pi-ai's until then and when the fetch fails", async () => {
  const net = network();
  const session = await openRouterSession(net);
  await until(() => listed(session, NEW), "the live list");
  expect(modelOption(session).values.find((v) => v.value === NEW)?.name).toBe(NEW_NAME);
  expect(listed(session, "openrouter/anthropic/claude-haiku-4.5")).toBe(true);
  // Only what the live list has.
  expect(listed(session, "openrouter/cohere/command-r-08-2024")).toBe(false);

  const down = network({ fail: true });
  const offline = await openRouterSession(down);
  await until(() => down.urls.length > 0, "the fetch");
  await Bun.sleep(10);
  expect(listed(offline, NEW)).toBe(false);
  expect(listed(offline, "openrouter/cohere/command-r-08-2024")).toBe(true);
});

test("a model only the live list knows is set with its live metadata", async () => {
  const session = await openRouterSession(network());
  await until(() => listed(session, NEW), "the live list");
  const sent: { contextWindow: number; input: string[] }[] = [];
  const fake = fakeModel([reply("ok")]);
  const streamFn: StreamFn = (model, context, options) => {
    sent.push({ contextWindow: model.contextWindow, input: model.input });
    return fake.streamFn(model, context, options);
  };
  const bound = await openRouterSession(network(), { model: NEW }, streamFn);
  await until(() => listed(bound, NEW), "the live list");
  await bound.setConfigOption?.("model", NEW);
  await bound.prompt({ text: "hi", app: "cli", pendingEvents: [], onUpdate: () => {} });
  expect(sent).toEqual([{ contextWindow: 1048576, input: ["text", "image"] }]);
});

// The host suffix

/** The models a session's requests were made with. */
function recorder(): { streamFn: StreamFn; models: { id: string; compat: unknown }[] } {
  const fake = fakeModel([reply("one"), reply("two")]);
  const models: { id: string; compat: unknown }[] = [];
  const streamFn: StreamFn = (model, context, options) => {
    models.push({ id: model.id, compat: structuredClone(model.compat) });
    return fake.streamFn(model, context, options);
  };
  return { streamFn, models };
}

const say = (session: AgentSession): Promise<unknown> =>
  session.prompt({ text: "hi", app: "cli", pendingEvents: [], onUpdate: () => {} });

test("a model value may end in @host; the request pins that host and nothing else", async () => {
  const seen = recorder();
  const session = await openRouterSession(network(), undefined, seen.streamFn);
  await until(() => listed(session, NEW), "the live list");
  await session.setConfigOption?.("model", `${NEW}@deepinfra/fp8`);
  expect(modelOption(session).current).toBe(`${NEW}@deepinfra/fp8`);
  // The listing shows the pinned model first, named with its host; the plain entry stays.
  expect(modelOption(session).values[0]).toEqual({
    value: `${NEW}@deepinfra/fp8`,
    name: `${NEW_NAME} @ deepinfra/fp8`,
  });
  expect(listed(session, NEW)).toBe(true);
  await say(session);
  expect(seen.models[0]).toEqual({
    id: "deepseek/deepseek-v9-flash-1017",
    compat: { openRouterRouting: { only: ["deepinfra/fp8"], allow_fallbacks: false } },
  });
  // Back to the plain model: no routing.
  await session.setConfigOption?.("model", NEW);
  expect(modelOption(session).current).toBe(NEW);
  await say(session);
  expect(seen.models[1]?.compat).toBeUndefined();
});

test("tags with several slashes, a host on a registry model and the stored setting all work", async () => {
  const seen = recorder();
  const haiku = "openrouter/anthropic/claude-haiku-4.5";
  const session = await openRouterSession(
    network(),
    { model: `${haiku}@deepinfra/fp8/priority` },
    seen.streamFn,
  );
  expect(modelOption(session).current).toBe(`${haiku}@deepinfra/fp8/priority`);
  await say(session);
  expect(seen.models[0]?.compat).toEqual({
    openRouterRouting: { only: ["deepinfra/fp8/priority"], allow_fallbacks: false },
  });
  // The shared registry object was copied, not changed.
  const shared = getModels("openrouter").find((m) => m.id === "anthropic/claude-haiku-4.5");
  expect(shared?.compat).toBeUndefined();
  // A bare openrouter id works with a host on an openrouter-configured neo, too.
  await session.setConfigOption?.("model", "anthropic/claude-haiku-4.5@wafer/mxfp4");
  expect(modelOption(session).current).toBe(`${haiku}@wafer/mxfp4`);
});

test("a host on a model of another provider is refused, with a readable error", async () => {
  const session = await openRouterSession(network());
  const failure = await errorMessage(
    session.setConfigOption?.("model", "deepseek/deepseek-v4-pro@deepinfra/fp8") ??
      Promise.resolve(),
  );
  expect(failure).toBe(
    '"@deepinfra/fp8" picks an OpenRouter host, but deepseek is not OpenRouter.',
  );
  expect(modelOption(session).current).toBe("openrouter/anthropic/claude-haiku-4.5");
});

test("an @ inside a registry model's own id is not a host", async () => {
  const session = await openRouterSession(network());
  await session.setConfigOption?.("model", "cloudflare-workers-ai/@cf/openai/gpt-oss-20b");
  expect(modelOption(session).current).toBe("cloudflare-workers-ai/@cf/openai/gpt-oss-20b");
});

// The host choice

test("the endpoints response becomes one option per host, after 'Any'", () => {
  const endpoints = parseEndpoints(ENDPOINTS);
  expect(endpoints.map((e) => e.tag)).toEqual(ids());
  expect(endpoints.find((e) => e.tag === "relace")).not.toHaveProperty("quantization");
  expect(endpointLabel(endpoints.find((e) => e.tag === "deepinfra/fp8") ?? fail())).toBe(
    "DeepInfra · fp8 · $0.098 / $0.29 per M · 1M ctx",
  );
  expect(endpointLabel(endpoints.find((e) => e.tag === "deepinfra/fp8/priority") ?? fail())).toBe(
    "DeepInfra priority · fp8 · $0.15 / $0.44 per M · 1M ctx",
  );
  expect(endpointLabel(endpoints.find((e) => e.tag === "relace") ?? fail())).toBe(
    "Relace · $0.050 / $0.15 per M · 1M ctx",
  );
});

function ids(): string[] {
  const data = (ENDPOINTS as { data: { endpoints: { tag: string }[] } }).data;
  return data.endpoints.map((e) => e.tag);
}

function fail(): never {
  throw new Error("missing endpoint");
}

test("setting an openrouter model without a host offers the hosts; 'Any' is the plain value", async () => {
  const net = network();
  const session = await openRouterSession(net);
  await until(() => listed(session, NEW), "the live list");
  await session.setConfigOption?.("model", NEW);
  const next = await session.followUp?.("model", NEW);
  expect(net.urls.at(-1)).toBe(
    "https://openrouter.ai/api/v1/models/deepseek/deepseek-v9-flash-1017/endpoints",
  );
  expect(next?.notice).toBeUndefined();
  expect(next?.choice?.title).toBe(`Host for ${NEW_NAME}`);
  const options = next?.choice?.options ?? [];
  expect(options[0]).toEqual({ value: NEW, name: "Any (OpenRouter chooses)" });
  expect(options).toHaveLength(1 + parseEndpoints(ENDPOINTS).length);
  expect(options).toContainEqual({
    value: `${NEW}@deepinfra/fp8`,
    name: "DeepInfra · fp8 · $0.098 / $0.29 per M · 1M ctx",
  });
  expect(options).toContainEqual({
    value: `${NEW}@wafer/mxfp4`,
    name: "Wafer · mxfp4 · $0.10 / $0.50 per M · 1M ctx",
  });
  // A host that is down says so.
  expect(options.find((o) => o.value.endsWith("@parasail/fp8"))?.description).toBe(
    "Having trouble right now",
  );
  // Picking "Any" sets the model without a suffix; picking a host sets it with one.
  await session.setConfigOption?.("model", options[0]?.value ?? "");
  expect(modelOption(session).current).toBe(NEW);
  await session.setConfigOption?.("model", `${NEW}@deepinfra/fp8`);
  expect(modelOption(session).current).toBe(`${NEW}@deepinfra/fp8`);
});

test("there is no host step for a model with a host, another provider or another setting", async () => {
  const net = network();
  const session = await openRouterSession(net);
  expect(await session.followUp?.("model", `${NEW}@deepinfra/fp8`)).toBeUndefined();
  expect(await session.followUp?.("model", "deepseek/deepseek-v4-pro")).toBeUndefined();
  expect(await session.followUp?.("model", "nope/x")).toBeUndefined();
  expect(await session.followUp?.("thought_level", "high")).toBeUndefined();
  expect(net.urls.filter((u) => u.endsWith("/endpoints"))).toEqual([]);
});

test("when the hosts can't be had the step is skipped with a notice", async () => {
  const session = await openRouterSession(network({ fail: true }));
  const failed = await session.followUp?.("model", "openrouter/anthropic/claude-haiku-4.5");
  expect(failed?.choice).toBeUndefined();
  expect(failed?.notice).toBe(
    "Could not list the hosts for Anthropic: Claude Haiku 4.5 (offline); OpenRouter chooses.",
  );
  const none = await openRouterSession(network({ endpoints: { data: { endpoints: [] } } }));
  const empty = await none.followUp?.("model", "openrouter/anthropic/claude-haiku-4.5");
  expect(empty?.choice).toBeUndefined();
  expect(empty?.notice).toContain("lists no hosts");
});

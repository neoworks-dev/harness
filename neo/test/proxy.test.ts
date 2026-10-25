import { afterEach, expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  getModels,
  getProviders,
  streamSimple,
  type Api,
  type Context,
  type KnownProvider,
  type Model,
} from "@mariozechner/pi-ai";
import { loadHarnessConfig } from "../src/config.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import { pinnedTo } from "../src/openrouter.ts";
import { proxiedModel, throughProxy } from "../src/proxy.ts";
import type { AgentUpdate } from "../src/agent.ts";
import { TEST_CONFIG } from "./fake-model.ts";
import { ofType, tempDir } from "./helpers.ts";

/** A request the fake proxy got. */
interface Seen {
  method: string;
  path: string;
  headers: Record<string, string>;
  body: Record<string, unknown>;
}

const servers: { stop: () => void }[] = [];
afterEach(() => {
  for (const server of servers.splice(0)) server.stop();
});

const CHUNKS = [
  { choices: [{ index: 0, delta: { role: "assistant", content: "Hello " } }] },
  { choices: [{ index: 0, delta: { content: "from the proxy" } }] },
  { choices: [{ index: 0, delta: {}, finish_reason: "stop" }] },
  {
    choices: [],
    usage: {
      prompt_tokens: 100,
      completion_tokens: 5,
      total_tokens: 105,
      prompt_tokens_details: { cached_tokens: 64 },
    },
  },
];

/** A proxy that records requests and answers like an OpenAI-compatible server. */
function fakeProxy(status = 200): { url: string; seen: Seen[] } {
  const seen: Seen[] = [];
  const server = Bun.serve({
    port: 0,
    async fetch(request) {
      const url = new URL(request.url);
      const text = await request.text();
      seen.push({
        method: request.method,
        path: url.pathname,
        headers: Object.fromEntries(request.headers),
        body: JSON.parse(text || "{}"),
      });
      if (status !== 200) {
        return Response.json({ error: { message: "nope", type: "bad" } }, { status });
      }
      const body = CHUNKS.map(
        (chunk) =>
          `data: ${JSON.stringify({ id: "x", object: "chat.completion.chunk", model: "m", ...chunk })}\n\n`,
      )
        .concat("data: [DONE]\n\n")
        .join("");
      return new Response(body, { headers: { "content-type": "text/event-stream" } });
    },
  });
  servers.push({ stop: () => void server.stop(true) });
  return { url: `http://127.0.0.1:${server.port}/v1`, seen };
}

const CONTEXT: Context = {
  systemPrompt: "You are a test.",
  messages: [{ role: "user", content: "hi", timestamp: 1 }],
  tools: [
    {
      name: "bash",
      description: "Run a command.",
      parameters: { type: "object", properties: { command: { type: "string" } } } as never,
    },
  ],
};

/** The body pi-ai builds for the model as it is, without sending it. */
async function bodyOf(model: Model<Api>, reasoning: "medium" | undefined): Promise<unknown> {
  let body: unknown;
  const stream = streamSimple(model, CONTEXT, {
    apiKey: "k",
    reasoning,
    sessionId: "session-1",
    onPayload: (payload) => {
      body = JSON.parse(JSON.stringify(payload));
      throw new Error("not sent");
    },
  });
  await stream.result();
  return body;
}

/** The body the fake proxy gets for the model, asked through throughProxy. */
async function bodyViaProxy(
  model: Model<Api>,
  reasoning: "medium" | undefined,
): Promise<{ body: unknown; seen: Seen }> {
  const proxy = fakeProxy();
  const [via, options] = throughProxy(
    model,
    { apiKey: "k", reasoning, sessionId: "session-1", maxRetries: 0 },
    {
      [model.provider]: proxy.url,
    },
  );
  await streamSimple(via, CONTEXT, options).result();
  const seen = proxy.seen[0];
  if (!seen) throw new Error("the proxy got nothing");
  return { body: seen.body, seen };
}

test("a proxied model asks the proxy, at the proxy's path, with its key", async () => {
  const model = getModels("deepseek")[0];
  if (!model) throw new Error("no deepseek model");
  const { seen } = await bodyViaProxy(model, undefined);
  expect(seen.method).toBe("POST");
  expect(seen.path).toBe("/v1/chat/completions");
  expect(seen.headers.authorization).toBe("Bearer k");
  expect(seen.body.model).toBe(model.id);
});

test("only the providers with a proxy are rerouted", () => {
  const [deepseek] = getModels("deepseek");
  const [openrouter] = getModels("openrouter");
  if (!deepseek || !openrouter) throw new Error("no models");
  const proxy = { openrouter: "http://localhost:8787/v1" };
  expect(proxiedModel(deepseek, proxy)).toBe(deepseek);
  expect(proxiedModel(deepseek, undefined)).toBe(deepseek);
  expect(proxiedModel(openrouter, proxy).baseUrl).toBe("http://localhost:8787/v1");
  // The registry's own object is left as it was.
  expect(openrouter.baseUrl).toBe("https://openrouter.ai/api/v1");
  const [same, options] = throughProxy(deepseek, undefined, proxy);
  expect(same).toBe(deepseek);
  expect(options).toBeUndefined();
});

/** A few models per provider: the first, one that reasons, one with compat settings. */
function sample(provider: KnownProvider): Model<Api>[] {
  const models: Model<Api>[] = getModels(provider);
  const picks = [models[0], models.find((m) => m.reasoning), models.find((m) => m.compat)];
  return [...new Set(picks.filter((m): m is Model<Api> => m !== undefined))];
}

test("a proxy changes nothing about the request body but where it goes", async () => {
  let compared = 0;
  for (const provider of getProviders()) {
    for (const model of sample(provider)) {
      // Cloudflare's base URL is made from the account's environment; pi-ai ignores the model's.
      if (model.api !== "openai-completions" || provider.startsWith("cloudflare")) continue;
      for (const reasoning of [undefined, "medium"] as const) {
        const direct = await bodyOf(model, reasoning);
        const { body } = await bodyViaProxy(model, reasoning);
        expect({ provider, model: model.id, reasoning, body }).toEqual({
          provider,
          model: model.id,
          reasoning,
          body: direct,
        });
        compared++;
      }
    }
  }
  // DeepSeek, OpenRouter, OpenAI and the other providers that detect from the URL.
  expect(compared).toBeGreaterThan(30);
});

test("an OpenRouter model pinned to a host keeps its provider routing through a proxy", async () => {
  const model = getModels("openrouter").find((m) => m.id.startsWith("deepseek/"));
  if (!model) throw new Error("no openrouter deepseek model");
  const pinned = pinnedTo(model, "deepinfra/fp8");
  const direct = (await bodyOf(pinned, "medium")) as Record<string, unknown>;
  expect(direct.provider).toEqual({ only: ["deepinfra/fp8"], allow_fallbacks: false });
  const { body } = await bodyViaProxy(pinned, "medium");
  expect(body).toEqual(direct);
});

test("DeepSeek's own API still gets what it understands: no store, system not developer", async () => {
  const model = getModels("deepseek")[0];
  if (!model) throw new Error("no deepseek model");
  const { body } = await bodyViaProxy(model, undefined);
  expect(body).not.toHaveProperty("store");
  const messages = (body as { messages: { role: string }[] }).messages;
  expect(messages[0]?.role).toBe("system");
});

test("a payload hook the options had still runs, before the body is put right", async () => {
  const model = getModels("openrouter")[0];
  if (!model) throw new Error("no model");
  const pinned = pinnedTo(model, "deepinfra/fp8");
  const proxy = fakeProxy();
  const [via, options] = throughProxy(
    pinned,
    {
      apiKey: "k",
      maxRetries: 0,
      onPayload: (payload) => ({ ...(payload as object), marker: 1 }),
    },
    { openrouter: proxy.url },
  );
  await streamSimple(via, CONTEXT, options).result();
  expect(proxy.seen[0]?.body).toMatchObject({ marker: 1, provider: { only: ["deepinfra/fp8"] } });
});

test("neo's turns go through the proxy and its usage still counts cached tokens", async () => {
  const proxy = fakeProxy();
  const config = { ...TEST_CONFIG, proxy: { deepseek: proxy.url } };
  const agent = createNeoAgent({ config });
  const cwd = await tempDir();
  const session = await agent.newSession({ sessionId: "s1", cwd, mcpServers: [] });
  const updates: AgentUpdate[] = [];
  const result = await session.prompt({
    text: "hi",
    app: "cli",
    pendingEvents: [],
    onUpdate: (update) => updates.push(update),
  });
  expect(result).toEqual({ stopReason: "end_turn" });
  expect(proxy.seen).toHaveLength(1);
  expect(proxy.seen[0]?.path).toBe("/v1/chat/completions");
  expect(proxy.seen[0]?.headers.authorization).toBe("Bearer test-key-not-real");
  const text = ofType(updates, "agent_message_chunk")
    .map((chunk) => chunk.text)
    .join("");
  expect(text).toBe("Hello from the proxy");
  // The upstream's usage, cached tokens included, arrives as it left the upstream.
  const assistant = ofType(updates, "harness_message")
    .map((u) => u.message as { role: string; usage?: { cacheRead: number; input: number } })
    .find((m) => m.role === "assistant");
  expect(assistant?.usage).toMatchObject({ cacheRead: 64, input: 36 });
});

test("generateText goes through the proxy too", async () => {
  const proxy = fakeProxy();
  const agent = createNeoAgent({ config: { ...TEST_CONFIG, proxy: { deepseek: proxy.url } } });
  const text = await agent.generateText?.({ system: "s", prompt: "p" });
  expect(text).toBe("Hello from the proxy");
  expect(proxy.seen).toHaveLength(1);
});

test("a proxy that fails is the turn's error, with its message", async () => {
  const proxy = fakeProxy(400);
  const agent = createNeoAgent({ config: { ...TEST_CONFIG, proxy: { deepseek: proxy.url } } });
  const cwd = await tempDir();
  const session = await agent.newSession({ sessionId: "s1", cwd, mcpServers: [] });
  const result = await session.prompt({
    text: "hi",
    app: "cli",
    pendingEvents: [],
    onUpdate: () => {},
  });
  expect(result.stopReason).toBe("error");
  expect(result.error).toContain("nope");
});

async function configWith(proxy: unknown): Promise<() => ReturnType<typeof loadHarnessConfig>> {
  const home = await tempDir();
  await mkdir(join(home, ".config", "neo"), { recursive: true });
  await writeFile(join(home, ".config", "neo", "config.json"), JSON.stringify({ proxy }));
  return () => loadHarnessConfig({ HOME: home });
}

test("config.json's proxy is a provider-keyed object of http(s) URLs", async () => {
  const good = await configWith({
    openrouter: "http://localhost:8787/v1",
    deepseek: "https://p.example/v1",
  });
  expect(good().proxy).toEqual({
    openrouter: "http://localhost:8787/v1",
    deepseek: "https://p.example/v1",
  });
  expect((await configWith({}))().proxy).toEqual({});
  expect(await configWith("http://localhost:8787/v1")).toThrow("proxy must be an object");
  expect(await configWith({ nonesuch: "http://x/v1" })).toThrow('unknown provider "nonesuch"');
  expect(await configWith({ openrouter: "localhost:8787" })).toThrow(
    "proxy.openrouter must be an http(s) URL",
  );
  expect(await configWith({ openrouter: "ftp://x/v1" })).toThrow("must be an http(s) URL");
  expect(await configWith({ openrouter: 5 })).toThrow("must be an http(s) URL");
});

test("no proxy setting, no proxy", () => {
  expect(loadHarnessConfig({ HOME: "/nonexistent" }).proxy).toBeUndefined();
});

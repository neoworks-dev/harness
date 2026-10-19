// Guests on the suite's API keys: `/model codex provider/model` starts the adapter (here the fake
// ACP agent) with the key; without such a pick it runs on its own login and gets nothing.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { rmSync } from "node:fs";
import { join } from "node:path";
import { setAccount } from "@suite/harness/auth";
import type { ChoiceOption, SessionEvent } from "@suite/protocol/types";
import { codexRoute } from "../src/guests/accounts.ts";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const KEY = "sk-or-test-key-000111222333";

/** How the fake agent reports a header value. */
function hashed(value: string): string {
  return new Bun.CryptoHasher("sha256").update(value).digest("hex").slice(0, 12);
}
const OTHER_KEY = "gsk-test-key-444555666777";

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

const cleanups: (() => Promise<void> | void)[] = [];
const saved: Record<string, string | undefined> = {};

/** Variables that would change what a guest gets: the developer's own, not the test's. */
const VARS = [
  "CODEX_CONFIG",
  "MODEL_PROVIDER",
  "OPENROUTER_API_KEY",
  "GROQ_API_KEY",
  "OPENAI_API_KEY",
  "XAI_API_KEY",
  "FIREWORKS_API_KEY",
  "AI_GATEWAY_API_KEY",
  "SUITE_API_KEY",
];

beforeEach(() => {
  for (const name of VARS) {
    saved[name] = process.env[name];
    delete process.env[name];
  }
});

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  for (const [name, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
});

interface Setup {
  daemon: TestDaemon;
  session: string;
  authPath: string;
  piAuthPath: string;
  logged: string[];
}

async function setup(): Promise<Setup> {
  const logged: string[] = [];
  const log = {
    info: (m: string) => logged.push(m),
    warn: (m: string) => logged.push(m),
    error: (m: string) => logged.push(m),
  };
  const daemon = await startTestDaemon({
    agent: new FakeAgent("main"),
    guests: { codex: { ...FAKE, title: "Codex", route: "codex" } },
    log,
  });
  cleanups.push(async () => {
    await daemon.stop();
    rmSync(daemon.dir, { recursive: true, force: true });
  });
  const session = daemon.sessions.create({}).id;
  return {
    daemon,
    session,
    authPath: join(daemon.dir, "data", "auth.json"),
    piAuthPath: join(daemon.dir, "data", "pi-auth.json"),
    logged,
  };
}

function events(s: Setup): SessionEvent[] {
  return s.daemon.store.read(s.session).map((r) => r.event);
}

/** What the agent said, chunk by chunk. */
function messages(s: Setup): string[] {
  const texts: string[] = [];
  for (const e of events(s)) if (e.type === "agent_message_chunk") texts.push(e.text);
  return texts;
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** Runs `/codex route` and returns what the adapter reported about how it was started. */
async function routeReport(s: Setup): Promise<{
  env: Record<string, string>;
  initialize: { clientCapabilities: Record<string, unknown> };
  authenticated?: { methodId: string; _meta: { gateway: Record<string, unknown> } };
}> {
  const before = events(s).filter((e) => e.type === "turn_ended").length;
  s.daemon.sessions.input(s.session, "cli", "/codex route");
  await until(() => events(s).filter((e) => e.type === "turn_ended").length > before, "the turn");
  return JSON.parse(messages(s).findLast((t) => t.startsWith("{")) ?? "null");
}

/** The options of the model card `/model codex` shows. */
async function modelCard(s: Setup): Promise<ChoiceOption[]> {
  await s.daemon.commands.run(s.session, "/model codex");
  await until(() => events(s).some((e) => e.type === "choice"), "the card");
  const card = events(s).findLast((e) => e.type === "choice");
  if (card?.type !== "choice") throw new Error("no card");
  await s.daemon.actions.run(
    {
      session: s.session,
      ref: "x",
      action: "suite.choose",
      args: { id: card.id, option: "small" },
    },
    "t",
  );
  return card.options;
}

describe("Codex on the suite's API keys", () => {
  test("without a pick the adapter gets nothing of ours", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    const report = await routeReport(s);
    expect(report.env).toEqual({});
    expect(report.initialize.clientCapabilities.auth).toBeUndefined();
    expect(report.authenticated).toBeUndefined();
  });

  test("/model codex lists the models of providers that have a key, and only those", async () => {
    const s = await setup();
    const plain = (await modelCard(s)).map((o) => o.id);
    expect(plain).toEqual(["small", "large"]);

    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await setAccount(s.piAuthPath, "groq", { type: "api_key", key: OTHER_KEY });
    // DeepSeek has no Responses API; ChatGPT is Codex's own login.
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: "ds-key-888999000111" });
    await setAccount(s.authPath, "openai-codex", {
      type: "oauth",
      access: "a-oauth-token-1",
      refresh: "r-oauth-token-1",
      expires: Date.now() + 3_600_000,
    });
    const options = await modelCard(s);
    const ids = options.map((o) => o.id);
    expect(ids.slice(0, 2)).toEqual(["small", "large"]);
    const providers = new Set(ids.slice(2).map((id) => id.split("/")[0]));
    expect(providers).toEqual(new Set(["openrouter", "groq"]));
    expect(options.find((o) => o.id.startsWith("openrouter/"))?.name).toStartWith("OpenRouter: ");
    expect(JSON.stringify(options)).not.toContain(KEY);
  });

  test("an environment variable counts as a key", async () => {
    const s = await setup();
    process.env.XAI_API_KEY = "xai-env-key-123456789";
    const ids = (await modelCard(s)).map((o) => o.id);
    expect(ids.some((id) => id.startsWith("xai/"))).toBe(true);
  });

  test("picking provider/model starts the adapter on the key through its gateway sign-in", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    expect(s.daemon.settings.get(s.session, "codex")).toEqual({
      model: "openrouter/openai/gpt-5-mini",
    });
    const report = await routeReport(s);
    expect(JSON.parse(report.env.CODEX_CONFIG ?? "null")).toEqual({ model: "openai/gpt-5-mini" });
    expect(report.initialize.clientCapabilities.auth).toEqual({ _meta: { gateway: true } });
    expect(report.authenticated).toEqual({
      methodId: "gateway",
      _meta: {
        gateway: {
          baseUrl: "https://openrouter.ai/api/v1",
          headers: { Authorization: hashed(`Bearer ${KEY}`) },
          providerName: "OpenRouter",
        },
      },
    });
    // The key is on the adapter's pipe only.
    expect(JSON.stringify(events(s))).not.toContain(KEY);
    expect(s.logged.join("\n")).not.toContain(KEY);
    const card = (await modelCard(s)).length;
    expect(card).toBeGreaterThan(2);
  });

  test("the conversation goes on when the adapter is restarted on a key", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    s.daemon.sessions.input(s.session, "cli", "/codex hello");
    await until(() => events(s).some((e) => e.type === "turn_ended"), "the first turn");
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    s.daemon.sessions.input(s.session, "cli", "/codex started");
    await until(() => events(s).filter((e) => e.type === "turn_ended").length === 2, "turn two");
    expect(messages(s)).toContain("started: resume");
  });

  test("a plain model after a key goes back to the adapter's own login", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    expect((await routeReport(s)).authenticated).toBeDefined();
    await s.daemon.commands.run(s.session, "/model codex large");
    const report = await routeReport(s);
    expect(report.env).toEqual({});
    expect(report.authenticated).toBeUndefined();
    expect(s.daemon.settings.get(s.session, "codex")).toEqual({ model: "large" });
  });

  test("a provider without a key is refused and nothing starts on it", async () => {
    const s = await setup();
    expect(
      s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini"),
    ).rejects.toThrow("No API key for openrouter. /login openrouter adds one.");
    // An OAuth login is not a key.
    await setAccount(s.authPath, "openai-codex", {
      type: "oauth",
      access: "a-oauth-token-1",
      refresh: "r-oauth-token-1",
      expires: Date.now() + 3_600_000,
    });
    // DeepSeek is not a Responses provider: its value is just an unknown model.
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: "ds-key-888999000111" });
    expect(
      s.daemon.commands.run(s.session, "/model codex deepseek/deepseek-v4-flash"),
    ).rejects.toThrow("no model deepseek/deepseek-v4-flash");
    expect(s.daemon.settings.get(s.session, "codex")).toEqual({});
  });

  test("the choice holds for the next start (a new session on the same last choice)", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    const next = s.daemon.sessions.create({}).id;
    const other = { ...s, session: next };
    const report = await routeReport(other);
    expect(JSON.parse(report.env.CODEX_CONFIG ?? "null")).toEqual({ model: "openai/gpt-5-mini" });
    expect(report.authenticated?.methodId).toBe("gateway");
  });

  test("a key that was removed since: the adapter starts on its own login", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    const { removeAccount } = await import("@suite/harness/auth");
    await removeAccount(s.authPath, "openrouter");
    const next = s.daemon.sessions.create({}).id;
    const report = await routeReport({ ...s, session: next });
    expect(report.env).toEqual({});
    expect(report.authenticated).toBeUndefined();
    expect(s.logged.join("\n")).toContain("No API key for openrouter");
  });
});

describe("Codex as the session's own harness", () => {
  test("/model provider/model restarts it on the key, in the same conversation", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/harness codex");
    s.daemon.sessions.input(s.session, "cli", "hello");
    await s.daemon.sessions.idle(s.session);
    await s.daemon.commands.run(s.session, "/model openrouter/openai/gpt-5-mini");
    s.daemon.sessions.input(s.session, "cli", "route");
    await s.daemon.sessions.idle(s.session);
    const report = JSON.parse(messages(s).findLast((t) => t.startsWith("{")) ?? "null");
    expect(JSON.parse(report.env.CODEX_CONFIG)).toEqual({ model: "openai/gpt-5-mini" });
    expect(report.authenticated.methodId).toBe("gateway");
    expect(JSON.stringify(events(s))).not.toContain(KEY);
  });
});

describe("codexRoute", () => {
  test("keeps what the user has in CODEX_CONFIG and sets the model", () => {
    process.env.CODEX_CONFIG = JSON.stringify({ approval_policy: "never", model: "old" });
    const route = codexRoute("groq", "openai/gpt-oss-120b", OTHER_KEY);
    expect(JSON.parse(route.env.CODEX_CONFIG ?? "null")).toEqual({
      approval_policy: "never",
      model: "openai/gpt-oss-120b",
    });
    expect(route.gateway?.baseUrl).toBe("https://api.groq.com/openai/v1");
    expect(route.secrets).toEqual([OTHER_KEY]);
  });
});

// Guests on neo's API keys: `/model codex provider/model` starts the adapter (here the fake
// ACP agent) with the key; without such a pick it runs on its own login and gets nothing.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { rmSync } from "node:fs";
import { join } from "node:path";
import { setAccount } from "@neoworks-dev/neo/auth";
import type { ChoiceOption, SessionEvent } from "@neoworks-dev/neo-protocol/types";
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
const DS_KEY = "ds-test-key-888999000111";
const ANTHROPIC_KEY = "sk-ant-api03-test-key-1234567";

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

const cleanups: (() => Promise<void> | void)[] = [];
const saved: Record<string, string | undefined> = {};

/** Variables that would change what a guest gets: the developer's own, not the test's. */
const VARS = [
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_OAUTH_TOKEN",
  "ANTHROPIC_PROFILE",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CODE_USE_BEDROCK",
  // Set when the tests run inside Claude Code.
  "CLAUDE_CODE_EXECUTABLE",
  "DEEPSEEK_API_KEY",
  "ZAI_API_KEY",
  "MOONSHOT_API_KEY",
  "MINIMAX_API_KEY",
  "CODEX_CONFIG",
  "MODEL_PROVIDER",
  "OPENROUTER_API_KEY",
  "GROQ_API_KEY",
  "OPENAI_API_KEY",
  "XAI_API_KEY",
  "FIREWORKS_API_KEY",
  "AI_GATEWAY_API_KEY",
  "NEO_API_KEY",
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
    guests: {
      codex: { ...FAKE, title: "Codex", route: "codex" },
      claude: {
        ...FAKE,
        title: "Claude Code",
        route: "claude",
        env: { CLAUDE_CODE_EXECUTABLE: "/usr/bin/claude" },
      },
    },
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

/** Runs `/<guest> route` and returns what the adapter reported about how it was started. */
async function routeReport(
  s: Setup,
  guest = "codex",
): Promise<{
  env: Record<string, string>;
  initialize: { clientCapabilities: Record<string, unknown> };
  authenticated?: { methodId: string; _meta: { gateway: Record<string, unknown> } };
}> {
  const before = events(s).filter((e) => e.type === "turn_ended").length;
  s.daemon.sessions.input(s.session, "cli", `/${guest} route`);
  await until(() => events(s).filter((e) => e.type === "turn_ended").length > before, "the turn");
  return JSON.parse(messages(s).findLast((t) => t.startsWith("{")) ?? "null");
}

/** The options of the model card `/model <guest>` shows (and answers it). */
async function modelCard(s: Setup, guest = "codex"): Promise<ChoiceOption[]> {
  const asked = events(s).filter((e) => e.type === "choice").length;
  await s.daemon.commands.run(s.session, `/model ${guest}`);
  await until(() => events(s).filter((e) => e.type === "choice").length > asked, "the card");
  const card = events(s).findLast((e) => e.type === "choice");
  if (card?.type !== "choice") throw new Error("no card");
  await s.daemon.actions.run(
    {
      session: s.session,
      ref: "x",
      action: "neo.choose",
      args: { id: card.id, option: "small" },
    },
    "t",
  );
  return card.options;
}

describe("Codex on neo's API keys", () => {
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
    expect(JSON.parse(report.env.CODEX_CONFIG ?? "null")).toEqual({
      model: "openai/gpt-5-mini",
      features: { multi_agent: false, multi_agent_v2: false },
    });
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
      s.daemon.commands.run(s.session, "/model codex deepseek/deepseek-flash"),
    ).rejects.toThrow("no model deepseek/deepseek-flash");
    expect(s.daemon.settings.get(s.session, "codex")).toEqual({});
  });

  test("the choice holds for the next start (a new session on the same last choice)", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    const next = s.daemon.sessions.create({}).id;
    const other = { ...s, session: next };
    const report = await routeReport(other);
    expect(JSON.parse(report.env.CODEX_CONFIG ?? "null")).toEqual({
      model: "openai/gpt-5-mini",
      features: { multi_agent: false, multi_agent_v2: false },
    });
    expect(report.authenticated?.methodId).toBe("gateway");
  });

  test("a key that was removed since: the adapter starts on its own login", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    const { removeAccount } = await import("@neoworks-dev/neo/auth");
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
    expect(JSON.parse(report.env.CODEX_CONFIG)).toEqual({
      model: "openai/gpt-5-mini",
      features: { multi_agent: false, multi_agent_v2: false },
    });
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
      // Codex's own subagents stay off: delegate hands work on.
      features: { multi_agent: false, multi_agent_v2: false },
    });
  });

  test("Codex's subagents are off unless the user turned them on", () => {
    process.env.CODEX_CONFIG = JSON.stringify({
      features: { multi_agent: true, web_search: true },
    });
    const route = codexRoute("groq", "openai/gpt-oss-120b", OTHER_KEY);
    expect(JSON.parse(route.env.CODEX_CONFIG ?? "null").features).toEqual({
      multi_agent: true,
      multi_agent_v2: false,
      web_search: true,
    });
    expect(route.gateway?.baseUrl).toBe("https://api.groq.com/openai/v1");
    expect(route.secrets).toEqual([OTHER_KEY]);
  });
});

describe("Claude Code on neo's API keys", () => {
  test("without a pick it keeps its own login: nothing of ours is set", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: DS_KEY });
    await setAccount(s.authPath, "anthropic", {
      type: "oauth",
      access: "a-oauth-token-1",
      refresh: "r-oauth-token-1",
      expires: Date.now() + 3_600_000,
    });
    const report = await routeReport(s, "claude");
    expect(report.env).toEqual({ CLAUDE_CODE_EXECUTABLE: "/usr/bin/claude" });
    expect(report.authenticated).toBeUndefined();
  });

  test("an Anthropic-compatible provider: base URL, token and its model for every alias", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: DS_KEY });
    // What the daemon was started with must not mix in.
    process.env.ANTHROPIC_API_KEY = "ambient-api-key";
    process.env.CLAUDE_CODE_OAUTH_TOKEN = "ambient-oauth";
    process.env.CLAUDE_CODE_USE_BEDROCK = "1";
    await s.daemon.commands.run(s.session, "/model claude deepseek/deepseek-flash");
    expect(s.daemon.settings.get(s.session, "claude")).toEqual({
      model: "deepseek/deepseek-flash",
    });
    const report = await routeReport(s, "claude");
    expect(report.env).toEqual({
      CLAUDE_CODE_EXECUTABLE: "/usr/bin/claude",
      ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic",
      ANTHROPIC_AUTH_TOKEN: DS_KEY,
      ANTHROPIC_MODEL: "deepseek-flash",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "deepseek-flash",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "deepseek-flash",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "deepseek-flash",
    });
    // Claude Code has no gateway sign-in to make; that is Codex's.
    expect(report.authenticated).toBeUndefined();
    expect(JSON.stringify(s.daemon.settings.get(s.session, "claude"))).not.toContain(DS_KEY);
    expect(s.logged.join("\n")).not.toContain(DS_KEY);
  });

  test("the other providers use their own endpoints", async () => {
    const s = await setup();
    const urls: Record<string, string> = {
      openrouter: "https://openrouter.ai/api",
      moonshotai: "https://api.moonshot.ai/anthropic",
      zai: "https://api.z.ai/api/anthropic",
      minimax: "https://api.minimax.io/anthropic",
    };
    for (const [provider, url] of Object.entries(urls)) {
      await setAccount(s.authPath, provider, { type: "api_key", key: `${provider}-key-1234567` });
      await s.daemon.commands.run(s.session, `/model claude ${provider}/some-model`);
      const report = await routeReport(s, "claude");
      expect(report.env.ANTHROPIC_BASE_URL).toBe(url);
      expect(report.env.ANTHROPIC_AUTH_TOKEN).toBe(`${provider}-key-1234567`);
      expect(report.env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("some-model");
    }
  });

  test("an Anthropic API key goes in ANTHROPIC_API_KEY, with no endpoint", async () => {
    const s = await setup();
    await setAccount(s.authPath, "anthropic", { type: "api_key", key: ANTHROPIC_KEY });
    process.env.ANTHROPIC_AUTH_TOKEN = "ambient-token";
    process.env.ANTHROPIC_BASE_URL = "https://gateway.example";
    await s.daemon.commands.run(s.session, "/model claude anthropic/claude-test-model");
    const report = await routeReport(s, "claude");
    expect(report.env).toEqual({
      CLAUDE_CODE_EXECUTABLE: "/usr/bin/claude",
      ANTHROPIC_API_KEY: ANTHROPIC_KEY,
      ANTHROPIC_MODEL: "claude-test-model",
    });
  });

  test("a Claude subscription is never handed over, however it is stored", async () => {
    const s = await setup();
    // pi's login, neo's login, a subscription token pasted as a key, and the variable.
    await setAccount(s.piAuthPath, "anthropic", {
      type: "oauth",
      access: "sk-ant-oat01-access-token",
      refresh: "sk-ant-ort01-refresh-token",
      expires: Date.now() + 3_600_000,
    });
    await setAccount(s.authPath, "anthropic", {
      type: "api_key",
      key: "sk-ant-oat01-pasted-token",
    });
    process.env.ANTHROPIC_OAUTH_TOKEN = "sk-ant-oat01-env-token";
    process.env.ANTHROPIC_API_KEY = "sk-ant-oat01-other-env-token";
    const models = (await modelCard(s, "claude")).map((o) => o.id);
    expect(models.some((id) => id.startsWith("anthropic/"))).toBe(false);
    expect(
      s.daemon.commands.run(s.session, "/model claude anthropic/claude-test-model"),
    ).rejects.toThrow("No API key for anthropic. /login anthropic adds one.");
    expect(JSON.stringify(s.daemon.settings.get(s.session, "claude"))).not.toContain("anthropic");
    // Nor does Codex get any of it.
    expect(
      s.daemon.commands.run(s.session, "/model codex anthropic/claude-test-model"),
    ).rejects.toThrow("no model anthropic/claude-test-model");
  });

  test("/model claude lists the models of Anthropic-compatible providers that have a key", async () => {
    const s = await setup();
    expect((await modelCard(s, "claude")).map((o) => o.id)).toEqual(["small", "large"]);
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: DS_KEY });
    await setAccount(s.authPath, "groq", { type: "api_key", key: OTHER_KEY });
    const ids = (await modelCard(s, "claude")).map((o) => o.id);
    expect(ids).toContain("deepseek/deepseek-flash");
    expect(ids).toContain("deepseek/deepseek-v4-pro");
    // Groq only has the OpenAI API, so it is Codex's.
    expect(ids.some((id) => id.startsWith("groq/"))).toBe(false);
    const codex = (await modelCard(s, "codex")).map((o) => o.id);
    expect(codex.some((id) => id.startsWith("groq/"))).toBe(true);
    expect(codex.some((id) => id.startsWith("deepseek/"))).toBe(false);
  });

  test("a plain model afterwards goes back to Claude Code's own login", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: DS_KEY });
    await s.daemon.commands.run(s.session, "/model claude deepseek/deepseek-v4-pro");
    expect((await routeReport(s, "claude")).env.ANTHROPIC_AUTH_TOKEN).toBe(DS_KEY);
    await s.daemon.commands.run(s.session, "/model claude large");
    expect((await routeReport(s, "claude")).env).toEqual({
      CLAUDE_CODE_EXECUTABLE: "/usr/bin/claude",
    });
  });

  test("Codex and Claude Code in one session pick their accounts apart", async () => {
    const s = await setup();
    await setAccount(s.authPath, "openrouter", { type: "api_key", key: KEY });
    await s.daemon.commands.run(s.session, "/model claude openrouter/some/model");
    expect((await routeReport(s, "codex")).authenticated).toBeUndefined();
    expect((await routeReport(s, "claude")).env.ANTHROPIC_BASE_URL).toBe(
      "https://openrouter.ai/api",
    );
    await s.daemon.commands.run(s.session, "/model codex openrouter/openai/gpt-5-mini");
    expect((await routeReport(s, "codex")).authenticated?.methodId).toBe("gateway");
    expect((await routeReport(s, "claude")).env.ANTHROPIC_MODEL).toBe("some/model");
  });

  test("a start that fails never says the key", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: DS_KEY });
    // An adapter that cannot start: the error text is cut of the key.
    const broken = await startTestDaemon({
      guests: {
        claude: {
          title: "Claude Code",
          command: ["bun", "-e", `console.error(${JSON.stringify(DS_KEY)}); process.exit(3)`],
          route: "claude",
          permissions: "ask",
        },
      },
      accounts: { authPath: s.authPath, piAuthPath: s.piAuthPath },
    });
    cleanups.push(() => broken.stop());
    const session = broken.sessions.create({}).id;
    broken.settings.set(session, "claude", "model", "deepseek/deepseek-flash");
    let message = "";
    try {
      await broken.guests.configOptions(session, "claude");
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    expect(message).toContain("exited");
    expect(message).not.toContain(DS_KEY);
  });
});

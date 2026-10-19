import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import {
  type OAuthCredentials,
  type OAuthLoginCallbacks,
  type OAuthProviderInterface,
  registerOAuthProvider,
  resetOAuthProviders,
} from "@mariozechner/pi-ai/oauth";
import { readAccounts, setAccount } from "@suite/harness/auth";
import type { Envelope, SessionEvent } from "@suite/protocol/types";
import { validateEnvelope } from "@suite/protocol/validate";
import type { ClientHandle } from "../src/hub.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";

const KEY = "sk-test-secret-key-1234567890";
const CODE = "the-pasted-code-abcdef-987654";
const ACCESS = "access-token-secret-aaaaaaaa";
const REFRESH = "refresh-token-secret-bbbbbbbb";

interface Client {
  received: Envelope[];
  handle: ClientHandle;
  /** Types text as input in the session. */
  type(text: string): void;
  /** The newest secret.request. */
  request(): Extract<Envelope, { topic: "secret.request" }>["payload"];
  /** Answers it; no value cancels. */
  answer(id: string, value?: string): void;
}

interface Setup {
  daemon: TestDaemon;
  session: string;
  /** Every line the daemon logged. */
  logged: string[];
  authPath: string;
  piAuthPath: string;
  client(): Client;
}

const daemons: TestDaemon[] = [];
const saved: Record<string, string | undefined> = {};

/** Variables that would change what /accounts says: set by the developer's shell, not by us. */
function keyVars(): string[] {
  return Object.keys(process.env).filter((k) => /(_API_KEY|_TOKEN|^AWS_|^GOOGLE_)/.test(k));
}

beforeEach(() => {
  for (const name of keyVars()) {
    saved[name] = process.env[name];
    delete process.env[name];
  }
});

afterEach(async () => {
  resetOAuthProviders();
  for (const name of keyVars()) delete process.env[name];
  for (const [name, value] of Object.entries(saved)) {
    if (value !== undefined) process.env[name] = value;
    delete saved[name];
  }
  for (const daemon of daemons.splice(0)) {
    await daemon.stop();
    rmSync(daemon.dir, { recursive: true, force: true });
  }
});

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 5));

async function until(check: () => boolean, what = "condition"): Promise<void> {
  for (let i = 0; i < 400; i++) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error(`timed out waiting for ${what}`);
}

async function setup(): Promise<Setup> {
  const logged: string[] = [];
  const log = {
    info: (m: string) => logged.push(m),
    warn: (m: string) => logged.push(m),
    error: (m: string) => logged.push(m),
  };
  const daemon = await startTestDaemon({ log });
  daemons.push(daemon);
  const session = daemon.sessions.create({}).id;
  const authPath = join(daemon.dir, "data", "auth.json");
  const piAuthPath = join(daemon.dir, "data", "pi-auth.json");
  const client = (): Client => {
    const received: Envelope[] = [];
    const handle = daemon.hub.connect({
      send(frame) {
        const result = validateEnvelope(JSON.parse(frame));
        if (!result.ok) throw new Error(`invalid frame: ${result.error}`);
        received.push(result.value);
      },
    });
    const send = (envelope: Record<string, unknown>): void =>
      handle.receive(JSON.stringify(envelope));
    send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
    return {
      received,
      handle,
      type: (text) =>
        send({ topic: "session.input", id: "i", payload: { session, app: "t", text } }),
      request: () => {
        const found = received.findLast((e) => e.topic === "secret.request");
        if (found?.topic !== "secret.request") throw new Error("no secret.request");
        return found.payload;
      },
      answer: (id, value) =>
        send({
          topic: "secret.answer",
          payload: value === undefined ? { id } : { id, value },
        }),
    };
  };
  return { daemon, session, logged, authPath, piAuthPath, client };
}

function events(s: Setup): SessionEvent[] {
  return s.daemon.store.read(s.session).map((r) => r.event);
}

function notices(s: Setup): string[] {
  const texts: string[] = [];
  for (const e of events(s)) if (e.type === "notice") texts.push(e.text);
  return texts;
}

function lastChoice(s: Setup): Extract<SessionEvent, { type: "choice" }> {
  const choice = events(s).findLast((e) => e.type === "choice");
  if (choice?.type !== "choice") throw new Error("no choice card");
  return choice;
}

async function pick(s: Setup, option: string): Promise<void> {
  await s.daemon.actions.run(
    {
      session: s.session,
      ref: "x",
      action: "suite.choose",
      args: { id: lastChoice(s).id, option },
    },
    "t",
  );
}

/** The secret is nowhere the daemon keeps or says things, apart from the auth file. */
function expectNoLeak(s: Setup, clients: Client[], ...secrets: string[]): void {
  const stored = JSON.stringify(s.daemon.store.read(s.session));
  const logged = s.logged.join("\n");
  for (const secret of secrets) {
    expect(stored).not.toContain(secret);
    expect(logged).not.toContain(secret);
    for (const c of clients) expect(JSON.stringify(c.received)).not.toContain(secret);
  }
}

function expectNoLeakOnDisk(s: Setup, ...secrets: string[]): void {
  const data = join(s.daemon.dir, "data");
  for (const name of readdirSync(data)) {
    if (name === "auth.json") continue;
    const path = join(data, name);
    if (!statSync(path).isFile()) continue;
    const bytes = readFileSync(path);
    for (const secret of secrets) expect(bytes.includes(secret)).toBe(false);
  }
}

function fakeOAuth(
  id: string,
  login: (callbacks: OAuthLoginCallbacks) => Promise<OAuthCredentials>,
): OAuthProviderInterface {
  const provider: OAuthProviderInterface = {
    id,
    name: `Fake ${id}`,
    login,
    refreshToken: (credentials) => Promise.resolve(credentials),
    getApiKey: (credentials) => credentials.access,
  };
  registerOAuthProvider(provider);
  return provider;
}

describe("/login with an API key", () => {
  test("asks for the key privately and stores it", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login key deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    expect(c.request()).toMatchObject({ session: s.session, title: "deepseek API key" });
    // The question is not in the session.
    expect(JSON.stringify(events(s))).not.toContain("deepseek API key");
    c.answer(c.request().id, `  ${KEY}\n`);
    await until(() => notices(s).length > 0);

    expect(notices(s)).toEqual(["Logged in to deepseek (API key)."]);
    expect(await readAccounts(s.authPath)).toEqual({ deepseek: { type: "api_key", key: KEY } });
    expect(statSync(s.authPath).mode & 0o777).toBe(0o600);
    expectNoLeak(s, [c], KEY);
    expectNoLeakOnDisk(s, KEY);
  });

  test("/login deepseek is the same", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id, KEY);
    await until(() => notices(s).length > 0);
    expect((await readAccounts(s.authPath)).deepseek).toEqual({ type: "api_key", key: KEY });
  });

  test("no argument: a card for the method, then one for the provider", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login");
    await until(() => events(s).some((e) => e.type === "choice"));
    const methods = lastChoice(s);
    expect(methods.title).toBe("Log in with");
    expect(methods.options.map((o) => o.name)).toEqual(
      expect.arrayContaining(["API key", "Claude Pro/Max", "ChatGPT", "GitHub Copilot"]),
    );
    await pick(s, "key");
    await until(() => events(s).filter((e) => e.type === "choice").length === 2);
    const providers = lastChoice(s);
    expect(providers.options.map((o) => o.id)).toContain("deepseek");
    await pick(s, "deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    expect(c.request().title).toBe("deepseek API key");
    c.answer(c.request().id, KEY);
    await until(() => notices(s).length > 0);
    expect(notices(s)).toEqual(["Logged in to deepseek (API key)."]);
    expectNoLeak(s, [c], KEY);
  });

  test("cancelling stores nothing", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id);
    await until(() => notices(s).length > 0);
    expect(notices(s)).toEqual(["Login to deepseek cancelled."]);
    expect(existsSync(s.authPath)).toBe(false);
  });

  test("an empty answer stores nothing", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id, "   ");
    await until(() => notices(s).length > 0);
    expect(existsSync(s.authPath)).toBe(false);
  });

  test("a variable that is set is mentioned", async () => {
    const s = await setup();
    process.env.DEEPSEEK_API_KEY = "from-the-environment";
    const c = s.client();
    c.type("/login deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id, KEY);
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toBe(
      "Logged in to deepseek (API key). DEEPSEEK_API_KEY is set and is used first.",
    );
    expectNoLeak(s, [c], "from-the-environment");
  });

  test("an unknown provider is refused without echoing the argument", async () => {
    const s = await setup();
    const c = s.client();
    c.type(`/login ${KEY}`);
    await flush();
    const error = c.received.findLast((e) => e.topic === "bus.error");
    expect(error).toMatchObject({ payload: { code: "bad_request" } });
    expect(JSON.stringify(c.received)).not.toContain(KEY);
    expect(c.received.some((e) => e.topic === "secret.request")).toBe(false);
  });

  test("a client that went away can't be asked", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/login deepseek");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.handle.closed();
    await until(() => notices(s).length > 0);
    expect(notices(s)).toEqual(["Login to deepseek cancelled."]);
    expect(existsSync(s.authPath)).toBe(false);
  });
});

describe("/login with an account", () => {
  test("shows the address, asks for the code, stores the credentials", async () => {
    const s = await setup();
    let seenCode = "";
    fakeOAuth("fake-oauth", async (cb) => {
      cb.onAuth({ url: "https://example.test/authorize?x=1", instructions: "Sign in there." });
      cb.onProgress?.("Waiting for the browser...");
      seenCode = await cb.onPrompt({ message: "Paste the code", placeholder: "code" });
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000, extra: "kept" };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    expect(c.request()).toMatchObject({ title: "Paste the code", placeholder: "code" });
    expect(notices(s)).toEqual([
      "Open this address to log in to Fake fake-oauth:\nhttps://example.test/authorize?x=1\nSign in there.",
      "Waiting for the browser...",
    ]);
    c.answer(c.request().id, CODE);
    await until(() => notices(s).length === 3);
    expect(seenCode).toBe(CODE);
    expect(notices(s)[2]).toBe("Logged in to Fake fake-oauth.");

    const stored = (await readAccounts(s.authPath))["fake-oauth"];
    expect(stored).toMatchObject({
      type: "oauth",
      access: ACCESS,
      refresh: REFRESH,
      extra: "kept",
    });
    expect(statSync(s.authPath).mode & 0o777).toBe(0o600);
    expectNoLeak(s, [c], ACCESS, REFRESH, CODE);
    expectNoLeakOnDisk(s, ACCESS, REFRESH, CODE);
  });

  test("a selection becomes a card", async () => {
    const s = await setup();
    let chosen: string | undefined;
    fakeOAuth("fake-oauth", async (cb) => {
      chosen = await cb.onSelect?.({
        message: "Which account?",
        options: [
          { id: "a", label: "Account A" },
          { id: "b", label: "Account B" },
        ],
      });
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000 };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => events(s).some((e) => e.type === "choice"));
    expect(lastChoice(s)).toMatchObject({
      title: "Which account?",
      options: [
        { id: "a", name: "Account A" },
        { id: "b", name: "Account B" },
      ],
    });
    await pick(s, "b");
    await until(() => notices(s).some((n) => n.startsWith("Logged in")));
    expect(chosen).toBe("b");
  });

  test("the browser finishing first closes the open code prompt", async () => {
    const s = await setup();
    let release: () => void = () => {};
    const browser = new Promise<void>((resolve) => (release = resolve));
    fakeOAuth("fake-oauth", async (cb) => {
      cb.onAuth({ url: "https://example.test/a" });
      void cb.onManualCodeInput?.();
      await browser;
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000 };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    const { id } = c.request();
    release();
    await until(() => notices(s).some((n) => n.startsWith("Logged in")));
    // The prompt is gone: a late answer finds nothing.
    c.answer(id, CODE);
    await flush();
    expect(c.received.findLast((e) => e.topic === "bus.error")).toMatchObject({
      payload: { code: "not_found" },
    });
    expectNoLeak(s, [c], CODE);
  });

  test("cancelling the code prompt cancels the login", async () => {
    const s = await setup();
    fakeOAuth("fake-oauth", async (cb) => {
      await cb.onPrompt({ message: "Paste the code" });
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000 };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id);
    await until(() => notices(s).length > 0);
    expect(notices(s)).toEqual(["Login cancelled."]);
    expect(existsSync(s.authPath)).toBe(false);
    // Free again: another login can start.
    c.type("/login fake-oauth");
    await until(() => c.received.filter((e) => e.topic === "secret.request").length === 2);
  });

  test("the client disconnecting aborts the login", async () => {
    const s = await setup();
    let aborted = false;
    fakeOAuth("fake-oauth", async (cb) => {
      cb.signal?.addEventListener("abort", () => (aborted = true));
      await cb.onManualCodeInput?.();
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000 };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.handle.closed();
    await until(() => notices(s).length > 0);
    expect(aborted).toBe(true);
    expect(notices(s)).toEqual(["Login cancelled."]);
    expect(existsSync(s.authPath)).toBe(false);
  });

  test("an error never carries what was pasted or received", async () => {
    const s = await setup();
    fakeOAuth("fake-oauth", async (cb) => {
      const code = await cb.onPrompt({ message: "Paste the code" });
      throw new Error(`token exchange rejected ${code} (refresh ${REFRESH})`);
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.answer(c.request().id, CODE);
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toStartWith("Login failed: token exchange rejected ***");
    expectNoLeak(s, [c], CODE);
    expect(existsSync(s.authPath)).toBe(false);
  });

  test("a second login to the same provider is refused while one runs", async () => {
    const s = await setup();
    fakeOAuth("fake-oauth", async (cb) => {
      await cb.onPrompt({ message: "Paste the code" });
      return { access: ACCESS, refresh: REFRESH, expires: Date.now() + 3_600_000 };
    });
    const c = s.client();
    c.type("/login fake-oauth");
    await until(() => c.received.some((e) => e.topic === "secret.request"));
    c.type("/login fake-oauth");
    await flush();
    expect(c.received.findLast((e) => e.topic === "bus.error")).toMatchObject({
      payload: { code: "bad_request" },
    });
  });

  test("the card lists a registered account provider by its own name", async () => {
    const s = await setup();
    fakeOAuth("fake-oauth", () => Promise.reject(new Error("unused")));
    const c = s.client();
    c.type("/login");
    await until(() => events(s).some((e) => e.type === "choice"));
    expect(lastChoice(s).options).toContainEqual(
      expect.objectContaining({ id: "fake-oauth", name: "Fake fake-oauth" }),
    );
  });
});

describe("/logout", () => {
  test("removes our login only; pi's file is pi's own", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: KEY });
    await setAccount(s.authPath, "openai", { type: "api_key", key: "other" });
    await setAccount(s.piAuthPath, "deepseek", { type: "api_key", key: "pi-key-000000" });
    const c = s.client();
    c.type("/logout deepseek");
    await until(() => notices(s).length > 0);
    expect(Object.keys(await readAccounts(s.authPath))).toEqual(["openai"]);
    expect(Object.keys(await readAccounts(s.piAuthPath))).toEqual(["deepseek"]);
    expect(notices(s)[0]).toContain("Logged out of deepseek.");
    expect(notices(s)[0]).toContain("pi's own");
    expect(notices(s)[0]).toContain(s.piAuthPath);
  });

  test("a variable is not touched, and says so", async () => {
    const s = await setup();
    process.env.DEEPSEEK_API_KEY = "from-the-environment";
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: KEY });
    const c = s.client();
    c.type("/logout deepseek");
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toContain("DEEPSEEK_API_KEY is set in the environment and stays.");
    expectNoLeak(s, [c], "from-the-environment", KEY);
  });

  test("nothing stored", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/logout deepseek");
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toStartWith("No login for deepseek");
    c.type("/logout");
    await until(() => notices(s).length > 1);
    expect(notices(s)[1]).toStartWith("No logins stored");
  });

  test("no argument: a card of what is stored", async () => {
    const s = await setup();
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: KEY });
    await setAccount(s.piAuthPath, "groq", { type: "api_key", key: "pi-key-000000" });
    const c = s.client();
    c.type("/logout");
    await until(() => events(s).some((e) => e.type === "choice"));
    expect(lastChoice(s).options).toEqual([
      { id: "deepseek", name: "API key" },
      { id: "groq", name: "API key (pi's file)" },
    ]);
    await pick(s, "deepseek");
    await until(() => notices(s).length > 0);
    expect(await readAccounts(s.authPath)).toEqual({});
    expect(JSON.stringify(events(s))).not.toContain(KEY);
  });
});

describe("/accounts", () => {
  test("lists where each login comes from, never the secret", async () => {
    const s = await setup();
    process.env.OPENROUTER_API_KEY = "env-secret-0000000";
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: KEY });
    await setAccount(s.authPath, "openai-codex", {
      type: "oauth",
      access: ACCESS,
      refresh: REFRESH,
      expires: Date.UTC(2099, 0, 2, 3, 4),
    });
    await setAccount(s.authPath, "github-copilot", {
      type: "oauth",
      access: ACCESS,
      refresh: REFRESH,
      expires: 1,
    });
    await setAccount(s.piAuthPath, "deepseek", { type: "api_key", key: "pi-key-000000" });
    await setAccount(s.piAuthPath, "groq", { type: "api_key", key: "pi-key-111111" });
    const c = s.client();
    c.type("/accounts");
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toBe(
      [
        "Logins (the first source is used):",
        "deepseek: API key from the suite's file, pi's file",
        "github-copilot: GitHub Copilot from the suite's file, token expired (renewed when used)",
        "groq: API key from pi's file",
        "openai-codex: ChatGPT from the suite's file, token until 2099-01-02 03:04",
        "openrouter: API key from OPENROUTER_API_KEY",
      ].join("\n"),
    );
    expectNoLeak(
      s,
      [c],
      KEY,
      ACCESS,
      REFRESH,
      "env-secret-0000000",
      "pi-key-000000",
      "pi-key-111111",
    );
  });

  test("an environment variable comes first", async () => {
    const s = await setup();
    process.env.DEEPSEEK_API_KEY = "env-secret-0000000";
    await setAccount(s.authPath, "deepseek", { type: "api_key", key: KEY });
    const c = s.client();
    c.type("/accounts");
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toContain("deepseek: API key from DEEPSEEK_API_KEY, the suite's file");
  });

  test("nothing", async () => {
    const s = await setup();
    const c = s.client();
    c.type("/accounts");
    await until(() => notices(s).length > 0);
    expect(notices(s)[0]).toBe("No logins. /login adds one.");
  });
});

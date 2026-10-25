// What the daemon announces to every client: where the agent lives (agent.focus, agent.move),
// connectors, sessions, the theme and file changes.
import { describe, expect, test } from "bun:test";
import type {
  AgentFocus,
  ConnectorInfo,
  FileChanged,
  SecretRequest,
} from "@neoworks-dev/neo-protocol/types";
import type { TestDaemon } from "@neoworks-dev/neo-daemon/testing";
import { createToolServer } from "@neoworks-dev/neo-sdk";
import { connect, fixedAddress, NeoError, TimeoutError, type Client } from "../src/index.ts";
import { cleanupAfterEach } from "./support/cleanup.ts";
import { FakeDaemon, THEME } from "./support/fake-daemon.ts";
import { sessionInfo } from "./support/fixtures.ts";
import { connectTo, startDaemon, startRig } from "./support/real-daemon.ts";
import { until } from "./support/wait.ts";

const later = cleanupAfterEach();

async function connectApp(
  daemon: TestDaemon,
  app: string,
  extra: { home?: boolean; pid?: number; context?: Record<string, string> } = {},
): Promise<Client> {
  const tools = await createToolServer({ name: app, tools: [] });
  later(() => tools.close());
  return connectTo(later, daemon, {
    client: app,
    presence: { app, kind: "app", open: true, mcp: tools.endpoint, ...extra },
  });
}

async function viewer(daemon: TestDaemon): Promise<{ client: Client; seen: AgentFocus[] }> {
  const client = await connectTo(later, daemon, { client: "viewer" });
  const seen: AgentFocus[] = [];
  client.focus.on((focus) => {
    if (focus) seen.push(focus);
  });
  return { client, seen };
}

async function fakeRig(): Promise<{ daemon: FakeDaemon; client: Client }> {
  const daemon = new FakeDaemon();
  later(() => daemon.stop());
  const client = await connect({ client: "test", address: fixedAddress(daemon) });
  later(() => client.close());
  return { daemon, client };
}

describe("agent focus", () => {
  test("the initial focus names the current session, which exists", async () => {
    const daemon = await startDaemon(later);
    const { client } = await viewer(daemon);
    const focus = await client.focus.waitFor();
    expect(focus.reason).toBe("initial");
    expect(client.focus.value).toEqual(focus);
    expect(focus.app).toBeUndefined();
    const sessions = await client.listSessions();
    expect(sessions.map((session) => session.id)).toContain(focus.session);
  });

  test("moving between apps and home is broadcast to everyone", async () => {
    const daemon = await startDaemon(later);
    const home = await connectApp(daemon, "home-app", { home: true, pid: 100 });
    await connectApp(daemon, "editor", { pid: 200 });
    const { client, seen } = await viewer(daemon);
    expect(await client.focus.waitFor()).toMatchObject({ app: "home-app", reason: "initial" });

    await home.moveAgent({ app: "editor" });
    await until(() => client.focus.value?.app === "editor", "move to editor");
    expect(client.focus.value).toMatchObject({ from: "home-app", reason: "user" });

    await client.moveAgent();
    await until(() => client.focus.value?.app === "home-app", "move home");
    expect(client.focus.value).toMatchObject({ from: "editor", reason: "user" });
    expect(seen.map((focus) => focus.app)).toEqual(["home-app", "editor", "home-app"]);
  });

  test("moving to a missing app or session is refused", async () => {
    const daemon = await startDaemon(later);
    const { client } = await viewer(daemon);
    await client.focus.waitFor();
    const noApp = await client.moveAgent({ app: "nope" }).catch((thrown: unknown) => thrown);
    expect(noApp).toBeInstanceOf(NeoError);
    expect(noApp).toMatchObject({ code: "not_found" });
    const noSession = await client
      .moveAgent({ session: "nope" })
      .catch((thrown: unknown) => thrown);
    expect(noSession).toMatchObject({ code: "not_found" });
  });

  test("the agent falls back home when its app closes", async () => {
    const daemon = await startDaemon(later);
    await connectApp(daemon, "home-app", { home: true });
    const editor = await connectApp(daemon, "editor");
    const { client } = await viewer(daemon);
    await client.focus.waitFor();
    await client.moveAgent({ app: "editor" });
    await until(() => client.focus.value?.app === "editor", "move to editor");
    await editor.close();
    await until(() => client.focus.value?.app === "home-app", "fallback");
    expect(client.focus.value).toMatchObject({ from: "editor", reason: "closed" });
  });

  test("moving with a session changes the current session for every client", async () => {
    const daemon = await startDaemon(later);
    const a = await viewer(daemon);
    const b = await viewer(daemon);
    const first = await a.client.focus.waitFor();
    const created = await a.client.createSession({ title: "next" });
    expect(created.id).not.toBe(first.session);

    await a.client.moveAgent({ session: created.id });
    await until(() => b.client.focus.value?.session === created.id, "session change");
    expect(b.client.focus.value?.reason).toBe("session");
    expect(a.client.focus.value?.session).toBe(created.id);
  });

  test("focus is sent again after a daemon restart", async () => {
    const first = await startDaemon(later);
    const { client, seen } = await viewer(first);
    await client.focus.waitFor();
    const before = seen.length;

    await first.stop();
    await client.state.waitFor(2000, (state) => state === "reconnecting");
    await startDaemon(later, first.dir);
    await until(() => seen.length > before, "focus after the reconnect", 8000);
    expect(seen.at(-1)?.reason).toBe("initial");
    expect(client.focus.value).toEqual(seen.at(-1));
  });

  test("waitFor rejects with TimeoutError when the daemon sends no focus", async () => {
    const { client } = await startRig(later);
    const silent = await fakeRig();
    // The fake daemon never sends agent.focus.
    const error = await silent.client.focus.waitFor(50).catch((thrown: unknown) => thrown);
    expect(error).toBeInstanceOf(TimeoutError);
    expect(String(error)).toContain("agent focus");
    expect(await client.focus.waitFor(2000)).toBeDefined();
  });
});

describe("theme", () => {
  test("the daemon's theme is kept and announced", async () => {
    const { client } = await fakeRig();
    expect(await client.theme.waitFor()).toEqual(THEME);
    expect(client.theme.value).toEqual(THEME);
  });

  test("a changed theme reaches listeners", async () => {
    const { daemon, client } = await fakeRig();
    await client.theme.waitFor();
    const seen: string[] = [];
    client.theme.on((theme) => {
      if (theme) seen.push(theme.name);
    });
    daemon.broadcast({ topic: "theme", payload: { ...THEME, name: "light" } });
    await until(() => seen.length === 1, "theme");
    expect(seen).toEqual(["light"]);
    expect(client.theme.value?.name).toBe("light");
  });
});

describe("connectors", () => {
  test("changes are broadcast, with home, pid and context", async () => {
    const daemon = await startDaemon(later);
    const { client } = await viewer(daemon);
    const lists: ConnectorInfo[][] = [];
    client.connectors.on((connectors) => lists.push(connectors));
    await connectApp(daemon, "home-app", { home: true, pid: 100 });
    const editor = await connectApp(daemon, "editor", { pid: 200, context: { project: "~/site" } });

    const find = (app: string): ConnectorInfo | undefined =>
      client.connectors.value.find((connector) => connector.app === app);
    await until(() => find("editor")?.connected === true, "editor connector");
    expect(find("home-app")).toMatchObject({ home: true, pid: 100, connected: true });
    expect(find("editor")).toMatchObject({ pid: 200, context: { project: "~/site" } });
    expect(lists.length).toBeGreaterThan(0);

    await editor.close();
    await until(() => find("editor")?.connected === false, "editor closed");
  });

  test("the list is fetched on connect", async () => {
    const daemon = await startDaemon(later);
    await connectApp(daemon, "home-app", { home: true, pid: 100 });
    const { client } = await viewer(daemon);
    await until(
      () => client.connectors.value.some((connector) => connector.app === "home-app"),
      "initial list",
    );
  });
});

describe("sessions", () => {
  test("the list is fetched on connect and again after a reconnect", async () => {
    const { daemon, client } = await fakeRig();
    await until(() => daemon.topics().includes("session.list"), "the first fetch");
    daemon.sessions = [sessionInfo("one"), sessionInfo("two")];
    await client.listSessions();
    expect(client.sessions.value.map((session) => session.id)).toEqual(["one", "two"]);

    daemon.sessions = [sessionInfo("one"), sessionInfo("two"), sessionInfo("three")];
    daemon.dropConnections();
    await client.sessions.waitFor(8000, (sessions) => sessions.length === 3);
  });

  test("session.changed replaces a known session and adds an unknown one", async () => {
    const { daemon, client } = await fakeRig();
    daemon.sessions = [sessionInfo("one"), sessionInfo("two")];
    await client.listSessions();
    const lists: string[][] = [];
    client.sessions.on((sessions) => lists.push(sessions.map((session) => session.id)));

    daemon.broadcast({
      topic: "session.changed",
      payload: sessionInfo("two", { state: "running", preview: "working" }),
    });
    await until(() => lists.length === 1, "change");
    expect(client.sessions.value[1]).toMatchObject({ id: "two", state: "running" });
    expect(lists[0]).toEqual(["one", "two"]);

    daemon.broadcast({ topic: "session.changed", payload: sessionInfo("three") });
    await until(() => lists.length === 2, "new session");
    expect(lists[1]).toEqual(["one", "two", "three"]);
  });

  test("session.removed drops the session from the list", async () => {
    const { daemon, client } = await fakeRig();
    daemon.sessions = [sessionInfo("one"), sessionInfo("two")];
    await client.listSessions();
    daemon.broadcast({ topic: "session.removed", payload: { id: "one" } });
    await client.sessions.waitFor(1000, (sessions) => sessions.length === 1);
    expect(client.sessions.value.map((session) => session.id)).toEqual(["two"]);
  });

  test("each change is a new array, and the old one is left as it was", async () => {
    const { daemon, client } = await fakeRig();
    daemon.sessions = [sessionInfo("one")];
    await client.listSessions();
    const before = client.sessions.value;
    daemon.broadcast({ topic: "session.changed", payload: sessionInfo("one", { title: "new" }) });
    await client.sessions.waitFor(1000, (sessions) => sessions[0]?.title === "new");
    expect(before[0]?.title).toBeUndefined();
  });

  test("a real daemon announces a session's turn: running, then idle with a preview", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    // Sessions you create yourself show up at once.
    expect(client.sessions.value.map((session) => session.id)).toContain(id);

    const states: string[] = [];
    client.sessions.on((sessions) => {
      const state = sessions.find((session) => session.id === id)?.state;
      if (state !== undefined && states.at(-1) !== state) states.push(state);
    });
    await client.input(id, "hello");
    await client.sessions.waitFor(5000, (sessions) =>
      sessions.some((session) => session.id === id && session.preview === "echo #1: hello"),
    );
    expect(states.at(-1)).toBe("idle");
  });
});

describe("file changes", () => {
  test("reach other clients, not the sender, and can be unsubscribed", async () => {
    const { daemon, client } = await startRig(later);
    const other = await connectTo(later, daemon, {
      client: "editor",
      presence: { app: "editor", kind: "app", open: true },
    });
    const seen: FileChanged[] = [];
    const own: FileChanged[] = [];
    const stop = client.onFileChanged((change) => seen.push(change));
    other.onFileChanged((change) => own.push(change));

    other.fileChanged("/tmp/a.txt");
    await until(() => seen.length === 1, "file.changed");
    expect(seen[0]).toEqual({ path: "/tmp/a.txt", app: "editor" });

    stop();
    other.fileChanged("/tmp/b.txt");
    await Bun.sleep(100);
    expect(seen).toHaveLength(1);
    expect(own).toHaveLength(0);
  });
});

describe("secret requests", () => {
  test("a request reaches the callback and the answer goes back with or without a value", async () => {
    const { daemon, client } = await fakeRig();
    const asked: SecretRequest[] = [];
    const stop = client.onSecretRequest((request) => asked.push(request));
    daemon.broadcast({
      topic: "secret.request",
      payload: { id: "s1", session: "a", title: "DeepSeek API key", placeholder: "sk-" },
    });
    await until(() => asked.length === 1, "secret.request");
    expect(asked[0]).toEqual({
      id: "s1",
      session: "a",
      title: "DeepSeek API key",
      placeholder: "sk-",
    });

    client.answerSecret("s1", "k-123");
    client.answerSecret("s2");
    await until(() => daemon.topics().filter((t) => t === "secret.answer").length === 2, "answers");
    const answers = daemon.received.filter((e) => e.topic === "secret.answer");
    expect(answers.map((e) => e.payload)).toEqual([{ id: "s1", value: "k-123" }, { id: "s2" }]);

    stop();
    daemon.broadcast({ topic: "secret.request", payload: { id: "s3", session: "a", title: "x" } });
    await Bun.sleep(50);
    expect(asked).toHaveLength(1);
  });
});

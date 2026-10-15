// Moving the agent between apps (agent.focus, agent.move, connectors.changed). These need the
// daemon's handoff support.
import { afterEach, describe, expect, test } from "bun:test";
import type { AgentFocus, ConnectorInfo } from "@suite/protocol/types";
import { startTestDaemon, type TestDaemon } from "@suite/daemon/testing";
import { SuiteClient, SuiteError } from "../src/client.ts";
import { createToolServer } from "../src/tool-server.ts";
import { until } from "./helpers.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function connectApp(
  daemon: TestDaemon,
  app: string,
  extra: { home?: boolean; pid?: number; context?: Record<string, string> } = {},
): Promise<SuiteClient> {
  const tools = createToolServer({ name: app, tools: [] });
  cleanups.push(() => tools.close());
  const client = await SuiteClient.connect({
    client: app,
    env: daemon.env,
    presence: {
      app,
      kind: "app",
      open: true,
      mcp: { url: tools.url, headers: tools.headers },
      ...extra,
    },
  });
  cleanups.push(() => client.close());
  return client;
}

async function viewer(daemon: TestDaemon): Promise<{ client: SuiteClient; seen: AgentFocus[] }> {
  const client = await SuiteClient.connect({ client: "viewer", env: daemon.env });
  cleanups.push(() => client.close());
  const seen: AgentFocus[] = [];
  client.onFocus((focus) => seen.push(focus));
  return { client, seen };
}

async function daemonRig(): Promise<TestDaemon> {
  const daemon = await startTestDaemon();
  cleanups.push(() => daemon.stop(200));
  return daemon;
}

describe("agent focus", () => {
  test("the initial focus names the current session, which exists", async () => {
    const daemon = await daemonRig();
    const { client } = await viewer(daemon);
    const focus = await client.waitForFocus();
    expect(focus.reason).toBe("initial");
    expect(client.focus).toEqual(focus);
    expect(focus.app).toBeUndefined();
    const sessions = await client.listSessions();
    expect(sessions.map((s) => s.id)).toContain(focus.session);
  });

  test("moving between apps and home is broadcast to everyone", async () => {
    const daemon = await daemonRig();
    const home = await connectApp(daemon, "home-app", { home: true, pid: 100 });
    await connectApp(daemon, "editor", { pid: 200 });
    const { client, seen } = await viewer(daemon);
    const initial = await client.waitForFocus();
    expect(initial).toMatchObject({ app: "home-app", reason: "initial" });

    await home.moveAgent({ app: "editor" });
    await until(() => client.focus?.app === "editor", "move to editor");
    expect(client.focus).toMatchObject({ from: "home-app", reason: "user" });

    await client.moveAgent();
    await until(() => client.focus?.app === "home-app", "move home");
    expect(client.focus).toMatchObject({ from: "editor", reason: "user" });
    expect(seen.map((f) => f.app)).toEqual(["home-app", "editor", "home-app"]);
  });

  test("moving to a missing app or session is refused", async () => {
    const daemon = await daemonRig();
    const { client } = await viewer(daemon);
    await client.waitForFocus();
    const noApp = await client.moveAgent({ app: "nope" }).catch((e: unknown) => e);
    expect(noApp).toBeInstanceOf(SuiteError);
    expect(noApp).toMatchObject({ code: "not_found" });
    const noSession = await client.moveAgent({ session: "nope" }).catch((e: unknown) => e);
    expect(noSession).toMatchObject({ code: "not_found" });
  });

  test("the agent falls back home when its app closes", async () => {
    const daemon = await daemonRig();
    await connectApp(daemon, "home-app", { home: true });
    const editor = await connectApp(daemon, "editor");
    const { client } = await viewer(daemon);
    await client.waitForFocus();
    await client.moveAgent({ app: "editor" });
    await until(() => client.focus?.app === "editor", "move to editor");
    await editor.close();
    await until(() => client.focus?.app === "home-app", "fallback");
    expect(client.focus).toMatchObject({ from: "editor", reason: "closed" });
  });

  test("moving with a session changes the current session for every client", async () => {
    const daemon = await daemonRig();
    const a = await viewer(daemon);
    const b = await viewer(daemon);
    const first = await a.client.waitForFocus();
    const created = await a.client.createSession({ title: "next" });
    expect(created.id).not.toBe(first.session);

    await a.client.moveAgent({ session: created.id });
    await until(() => b.client.focus?.session === created.id, "session change");
    expect(b.client.focus?.reason).toBe("session");
    expect(a.client.focus?.session).toBe(created.id);
  });

  test("focus is sent again after a daemon restart", async () => {
    const first = await startTestDaemon();
    let daemon = first;
    cleanups.push(() => daemon.stop(200));
    const client = await SuiteClient.connect({ client: "viewer", env: first.env });
    cleanups.push(() => client.close());
    const seen: AgentFocus[] = [];
    client.onFocus((focus) => seen.push(focus));
    await client.waitForFocus();
    const changes: boolean[] = [];
    client.onConnectionChange((c) => changes.push(c));

    const before = seen.length;
    await first.stop(200);
    daemon = await startTestDaemon({ dir: first.dir });
    await until(() => changes.join() === "false,true", "reconnect");
    await until(() => seen.length > before, "focus after reconnect");
    expect(seen.at(-1)?.reason).toBe("initial");
    expect(client.focus).toEqual(seen.at(-1));
  });
});

describe("connectors", () => {
  test("changes are broadcast, with home, pid and context", async () => {
    const daemon = await daemonRig();
    const { client } = await viewer(daemon);
    const lists: ConnectorInfo[][] = [];
    client.onConnectors((connectors) => lists.push(connectors));
    await connectApp(daemon, "home-app", { home: true, pid: 100 });
    const editor = await connectApp(daemon, "editor", { pid: 200, context: { project: "~/site" } });

    const find = (app: string): ConnectorInfo | undefined =>
      client.connectors.find((c) => c.app === app);
    await until(() => find("editor")?.connected === true, "editor connector");
    expect(find("home-app")).toMatchObject({ home: true, pid: 100, connected: true });
    expect(find("editor")).toMatchObject({ pid: 200, context: { project: "~/site" } });
    expect(lists.length).toBeGreaterThan(0);

    await editor.close();
    await until(() => find("editor")?.connected === false, "editor closed");
  });

  test("the list is fetched on connect", async () => {
    const daemon = await daemonRig();
    await connectApp(daemon, "home-app", { home: true, pid: 100 });
    const { client } = await viewer(daemon);
    await until(() => client.connectors.some((c) => c.app === "home-app"), "initial list");
  });
});

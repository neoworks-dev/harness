import { describe, expect, test } from "bun:test";
import type { AppPresence } from "@neoworks-dev/neo-protocol/types";
import { createToolServer } from "@neoworks-dev/neo-sdk";
import {
  connect,
  DisconnectedError,
  fixedAddress,
  SuiteError,
  TimeoutError,
} from "../src/index.ts";
import { cleanupAfterEach } from "./support/cleanup.ts";
import { FakeDaemon } from "./support/fake-daemon.ts";
import { Recorder } from "./support/recorder.ts";
import {
  addressIn,
  connectTo,
  emptyDaemonDir,
  startDaemon,
  startRig,
} from "./support/real-daemon.ts";
import { rejection, until } from "./support/wait.ts";

const later = cleanupAfterEach();

describe("requests", () => {
  test("create and list sessions", async () => {
    const { client } = await startRig(later);
    // The daemon always has a current session.
    const initial = (await client.listSessions()).map((session) => session.id);
    expect(initial).toHaveLength(1);
    const created = await client.createSession({ title: "t" });
    expect(created.title).toBe("t");
    const listed = (await client.listSessions()).map((session) => session.id);
    expect(listed.sort()).toEqual([...initial, created.id].sort());
  });

  test("sessionSettings lists the harnesses of a session; an unknown session is refused", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    const { harnesses } = await client.sessionSettings(id);
    // The test daemon has the echo agent only: no guests, nothing to set.
    expect(harnesses).toHaveLength(1);
    expect(harnesses[0]).toMatchObject({ current: true, options: [] });
    expect(harnesses[0]?.error).toBeUndefined();
    const error = await rejection(client.sessionSettings("s_nope"));
    expect(error).toBeInstanceOf(SuiteError);
  });

  test("input comes from the presence's app, else the client name, else the app you give", async () => {
    const { daemon, client } = await startRig(later);
    const { id } = await client.createSession();
    const events = new Recorder();
    client.subscribe(id, events);

    await client.input(id, "a");
    const named = await connectTo(later, daemon, {
      client: "other",
      presence: { app: "editor", kind: "app", open: true },
    });
    await until(() => events.turnsEnded() === 1, "first turn");
    await named.input(id, "b");
    await client.input(id, "c", { app: "photo" });
    await until(() => events.turnsEnded() === 3, "all turns");

    const apps: string[] = [];
    for (const { event } of events.records) {
      if (event.type === "user_message") apps.push(event.app);
    }
    expect(apps).toEqual(["test", "editor", "photo"]);
  });

  test("cancel ends a running turn", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    const events = new Recorder();
    client.subscribe(id, events);
    await client.input(id, "wait:10000 slow");
    await until(() => events.types().includes("user_message"), "turn start");
    await client.cancel(id);
    await until(() => events.turnsEnded() === 1, "turn end");
    expect(events.turnEnds()).toEqual(["cancelled"]);
  });

  test("a request the daemon refuses rejects with SuiteError and its code", async () => {
    const { client } = await startRig(later);
    const error = await client.input("nope", "x").catch((thrown: unknown) => thrown);
    expect(error).toBeInstanceOf(SuiteError);
    expect(error).toMatchObject({ code: "not_found" });
  });

  test("a request the daemon never answers rejects with TimeoutError", async () => {
    const daemon = new FakeDaemon();
    later(() => daemon.stop());
    daemon.silentOn.add("ui.action");
    const client = await connect({
      client: "test",
      address: fixedAddress(daemon),
      requestTimeoutMs: 100,
    });
    later(() => client.close());

    const error = await client.action("s1", "ref", "do.it").catch((thrown: unknown) => thrown);
    expect(error).toBeInstanceOf(TimeoutError);
    expect(String(error)).toContain("ui.action");
    // Other requests are not affected.
    expect(await client.listSessions()).toEqual([]);
  });
});

describe("connecting", () => {
  test("rejects with DisconnectedError when no daemon is running", async () => {
    const attempt = connect({
      client: "test",
      address: addressIn(emptyDaemonDir()),
      reconnect: false,
    });
    expect(await rejection(attempt)).toBeInstanceOf(DisconnectedError);
    expect(String(await rejection(attempt))).toContain("not running");
  });

  test("waitForDaemon resolves once the daemon starts", async () => {
    const dir = emptyDaemonDir();
    const connecting = connect({ client: "test", address: addressIn(dir), waitForDaemon: true });
    await Bun.sleep(400);
    await startDaemon(later, dir);
    const client = await connecting;
    later(() => client.close());
    expect(await client.listSessions()).toHaveLength(1);
  });
});

describe("close", () => {
  test("marks the app closed, ends the connection and rejects later calls", async () => {
    const daemon = new FakeDaemon();
    later(() => daemon.stop());
    const presence: AppPresence = { app: "demo", kind: "app", open: true };
    const client = await connect({ client: "demo", address: fixedAddress(daemon), presence });
    const states: string[] = [];
    client.state.on((state) => states.push(state));

    await client.close();
    await client.close();
    const registered: boolean[] = [];
    for (const frame of daemon.received) {
      if (frame.topic === "app.presence") registered.push(frame.payload.open);
    }
    expect(registered).toEqual([true, false]);
    expect(states).toEqual(["closed"]);
    expect(await rejection(client.listSessions())).toBeInstanceOf(DisconnectedError);
    expect(() => client.fileChanged("/tmp/a")).toThrow(DisconnectedError);
  });
});

describe("daemon restart", () => {
  test("the state goes reconnecting, then open; requests meanwhile are refused", async () => {
    const first = await startDaemon(later);
    const client = await connectTo(later, first);
    const states: string[] = [];
    client.state.on((state) => states.push(state));

    await first.stop();
    await client.state.waitFor(2000, (state) => state === "reconnecting");
    expect(await rejection(client.listSessions())).toBeInstanceOf(DisconnectedError);

    await startDaemon(later, first.dir);
    await client.state.waitFor(8000, (state) => state === "open");
    expect(states).toEqual(["reconnecting", "open"]);
    expect(await client.listSessions()).toHaveLength(1);
  });

  test("with reconnect: false the client closes and stays down", async () => {
    const first = await startDaemon(later);
    const client = await connectTo(later, first, { reconnect: false });
    await first.stop();
    await client.state.waitFor(2000, (state) => state === "closed");
    await startDaemon(later, first.dir);
    await Bun.sleep(700);
    expect(client.state.value).toBe("closed");
  });

  test("a request in flight when the connection drops rejects with DisconnectedError", async () => {
    const daemon = new FakeDaemon();
    later(() => daemon.stop());
    daemon.silentOn.add("ui.action");
    const client = await connect({ client: "test", address: fixedAddress(daemon) });
    later(() => client.close());

    const inFlight = client.action("s1", "ref", "do.it").catch((thrown: unknown) => thrown);
    await until(() => daemon.topics().includes("ui.action"), "the request to arrive");
    daemon.dropConnections();
    expect(await inFlight).toBeInstanceOf(DisconnectedError);
  });
});

// These need the daemon's MCP gateway (app.presence, connectors.list).
describe("presence (needs the gateway)", () => {
  const presenceFor = (url: string, headers: Record<string, string>): AppPresence => ({
    app: "demo",
    kind: "app",
    title: "Demo",
    open: true,
    mcp: { url, headers },
  });

  test("registered tools show up in listConnectors, and survive a restart", async () => {
    const tools = await createToolServer({
      name: "demo",
      tools: [
        {
          name: "ping",
          description: "Replies pong",
          inputSchema: { type: "object", properties: {} },
          handler: () => Promise.resolve("pong"),
        },
      ],
    });
    later(() => tools.close());
    const first = await startDaemon(later);
    const client = await connectTo(later, first, {
      client: "demo",
      presence: presenceFor(tools.url, tools.headers),
    });

    const toolNames = async (): Promise<string[]> => {
      const connectors = await client.listConnectors();
      return connectors
        .filter((connector) => connector.app === "demo" && connector.connected)
        .flatMap((connector) => connector.tools.map((tool) => tool.name));
    };
    expect(await toolNames()).toEqual(["demo.ping"]);

    await first.stop();
    await client.state.waitFor(2000, (state) => state === "reconnecting");
    await startDaemon(later, first.dir);
    await client.state.waitFor(8000, (state) => state === "open");
    expect(await toolNames()).toEqual(["demo.ping"]);

    // Closing the app marks it disconnected but known.
    await client.setPresence({ ...presenceFor(tools.url, tools.headers), open: false });
    const after = (await client.listConnectors()).find((connector) => connector.app === "demo");
    expect(after?.connected).toBe(false);
  });

  test("setContext updates the app's context and is kept for reconnects", async () => {
    const daemon = await startDaemon(later);
    const app = await connectTo(later, daemon, {
      client: "code-editor",
      presence: { app: "code-editor", kind: "app", open: true, context: { line: "1" } },
    });

    await app.setContext({ file: "/site/a.ts", line: "40" });
    const listed = daemon.connectors.list().find((connector) => connector.app === "code-editor");
    expect(listed?.context).toEqual({ file: "/site/a.ts", line: "40" });

    await daemon.stop();
    await app.state.waitFor(2000, (state) => state === "reconnecting");
    const restarted = await startDaemon(later, daemon.dir);
    await app.state.waitFor(8000, (state) => state === "open");
    const kept = restarted.connectors.list().find((connector) => connector.app === "code-editor");
    expect(kept?.context).toEqual({ file: "/site/a.ts", line: "40" });
  });

  test("setContext without a presence says to set one first", async () => {
    const { client } = await startRig(later);
    expect(String(await rejection(client.setContext({ a: "b" })))).toContain("setPresence");
  });
});

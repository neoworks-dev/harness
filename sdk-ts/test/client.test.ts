import { afterEach, describe, expect, test } from "bun:test";
import type { AppPresence } from "@suite/protocol/types";
import { startTestDaemon, type TestDaemon } from "@suite/daemon/testing";
import { DisconnectedError, SuiteClient, SuiteError } from "../src/client.ts";
import { createToolServer } from "../src/tool-server.ts";
import { emptyEnv, Recorder, start, until } from "./helpers.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function rig(): Promise<{ daemon: TestDaemon; client: SuiteClient }> {
  const r = await start();
  cleanups.push(() => r.daemon.stop(200));
  cleanups.push(() => r.client.close());
  return r;
}

describe("sessions", () => {
  test("create, list, subscribe replays then follows live, input gets an echo", async () => {
    const { client } = await rig();
    expect(await client.listSessions()).toEqual([]);
    const created = await client.createSession({ title: "t" });
    expect(created.title).toBe("t");
    expect((await client.listSessions()).map((s) => s.id)).toEqual([created.id]);

    await client.input(created.id, "one");
    const first = new Recorder();
    // Subscribing after (or during) the first turn replays what is there.
    const sub = client.subscribe(created.id, first);
    await until(() => first.live === 1 && first.turnsEnded() === 1, "replay");
    expect(first.types()[0]).toBe("user_message");
    expect(first.text()).toBe("echo #1: one");

    await client.input(created.id, "two", { app: "photo" });
    await until(() => first.turnsEnded() === 2, "second turn");
    expect(first.text()).toBe("echo #1: oneecho #2: two");
    const user = first.records.find((r) => r.event.type === "user_message" && r.seq > 3);
    expect(user?.event).toMatchObject({ app: "photo", text: "two" });
    expect(first.seqs).toEqual(first.seqs.map((_, i) => i + 1));
    sub.close();
  });

  test("fromSeq skips older events; input defaults to the presence app, else the client name", async () => {
    const { client } = await rig();
    const { id } = await client.createSession();
    await client.input(id, "a");
    const all = new Recorder();
    client.subscribe(id, all);
    await until(() => all.turnsEnded() === 1, "first turn");
    const last = all.seqs.at(-1) ?? 0;

    const tail = new Recorder();
    client.subscribe(id, { fromSeq: last + 1, onEvent: tail.onEvent });
    await client.input(id, "b");
    await until(() => tail.turnsEnded() === 1, "second turn");
    expect(tail.seqs[0]).toBe(last + 1);
    expect(tail.records[0]?.event).toMatchObject({ type: "user_message", app: "test" });
  });

  test("cancel ends a running turn", async () => {
    const { client } = await rig();
    const { id } = await client.createSession();
    const rec = new Recorder();
    client.subscribe(id, rec);
    await client.input(id, "wait:10000 slow");
    await until(() => rec.types().includes("user_message"), "turn start");
    await client.cancel(id);
    await until(() => rec.turnsEnded() === 1, "turn end");
    const ended = rec.records.find((r) => r.event.type === "turn_ended");
    expect(ended?.event).toMatchObject({ stopReason: "cancelled" });
  });

  test("closing a subscription stops its events", async () => {
    const { client } = await rig();
    const { id } = await client.createSession();
    const rec = new Recorder();
    const sub = client.subscribe(id, rec);
    await until(() => rec.live === 1, "live");
    sub.close();
    await client.input(id, "x");
    await Bun.sleep(150);
    expect(rec.records).toEqual([]);
  });

  test("a bad request rejects with the daemon's error", async () => {
    const { client } = await rig();
    const err = await client.input("nope", "x").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(SuiteError);
    expect(err).toMatchObject({ code: "not_found" });
  });

  test("an unknown session reaches the subscription's onError", async () => {
    const { client } = await rig();
    const errors: Error[] = [];
    client.subscribe("nope", { onEvent: () => {}, onError: (e) => errors.push(e) });
    await until(() => errors.length === 1, "error");
    expect(errors[0]).toMatchObject({ code: "not_found" });
  });
});

describe("connecting", () => {
  test("rejects when no daemon is running", async () => {
    const attempt = SuiteClient.connect({ client: "test", env: emptyEnv(), reconnect: false });
    const err = await attempt.catch((e: unknown) => e);
    expect(err).toBeInstanceOf(DisconnectedError);
    expect(String(err)).toContain("not running");
  });

  test("waitForDaemon resolves once the daemon starts", async () => {
    const env = emptyEnv();
    const dir = env.SUITE_SOCKET?.replace(/\/bus\.sock$/, "") ?? "";
    const connecting = SuiteClient.connect({ client: "test", env, waitForDaemon: true });
    await Bun.sleep(400);
    const daemon = await startTestDaemon({ dir });
    cleanups.push(() => daemon.stop(200));
    const client = await connecting;
    cleanups.push(() => client.close());
    expect(await client.listSessions()).toEqual([]);
  });
});

describe("daemon restart", () => {
  test("resumes subscriptions without gaps or duplicates", async () => {
    const first = await startTestDaemon();
    let daemon = first;
    cleanups.push(() => daemon.stop(200));
    const client = await SuiteClient.connect({ client: "test", env: first.env });
    cleanups.push(() => client.close());

    const changes: boolean[] = [];
    client.onConnectionChange((c) => changes.push(c));
    const { id } = await client.createSession();
    const rec = new Recorder();
    client.subscribe(id, rec);
    await client.input(id, "before");
    await until(() => rec.turnsEnded() === 1, "first turn");
    const seen = rec.records.length;

    // A request in flight when the daemon goes away fails instead of hanging.
    const inFlight = client.input(id, "wait:10000 lost");
    await until(() => rec.types().includes("user_message") && rec.records.length > seen, "start");
    await first.stop(200);
    await inFlight.catch(() => {});
    await until(() => changes.includes(false), "disconnect");
    const down = await client.listSessions().catch((e: unknown) => e);
    expect(down).toBeInstanceOf(DisconnectedError);

    daemon = await startTestDaemon({ dir: first.dir });
    await until(() => client.isConnected, "reconnect");
    expect(changes).toEqual([false, true]);
    expect(rec.live).toBe(2);

    await client.input(id, "after");
    await until(() => rec.turnsEnded() === 3, "turns after restart");
    // Gap-free and duplicate-free, including the turn the restart interrupted.
    expect(rec.seqs).toEqual(rec.seqs.map((_, i) => i + 1));
    expect(rec.text()).toContain("echo #3: after");
    const reasons = rec.turnEnds();
    expect(reasons).toHaveLength(3);
    // The turn the restart interrupted ended one way or the other, and is in the replay.
    expect(["cancelled", "error"]).toContain(reasons[1] ?? "");
  });

  test("with reconnect: false the client stays down", async () => {
    const first = await startTestDaemon();
    let daemon = first;
    cleanups.push(() => daemon.stop(200));
    const client = await SuiteClient.connect({ client: "test", env: first.env, reconnect: false });
    await first.stop(200);
    await until(() => !client.isConnected, "disconnect");
    daemon = await startTestDaemon({ dir: first.dir });
    await Bun.sleep(700);
    expect(client.isConnected).toBe(false);
    await client.close();
  });
});

// These need the daemon's MCP gateway (app.presence, connectors.list).
describe("presence (needs the gateway)", () => {
  const presence = (url: string, headers: Record<string, string>): AppPresence => ({
    app: "demo",
    kind: "app",
    title: "Demo",
    open: true,
    mcp: { url, headers },
  });

  test("registered tools show up in listConnectors, and survive a restart", async () => {
    const tools = createToolServer({
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
    cleanups.push(() => tools.close());
    const first = await startTestDaemon();
    let daemon = first;
    cleanups.push(() => daemon.stop(200));
    const client = await SuiteClient.connect({
      client: "demo",
      env: first.env,
      presence: presence(tools.url, tools.headers),
    });
    cleanups.push(() => client.close());

    const find = async (): Promise<string[]> => {
      const connectors = await client.listConnectors();
      return connectors
        .filter((c) => c.app === "demo" && c.connected)
        .flatMap((c) => c.tools.map((t) => t.name));
    };
    expect(await find()).toEqual(["demo.ping"]);

    const changes: boolean[] = [];
    client.onConnectionChange((connected) => changes.push(connected));
    await first.stop(200);
    daemon = await startTestDaemon({ dir: first.dir });
    await until(() => changes.join() === "false,true", "reconnect");
    expect(await find()).toEqual(["demo.ping"]);

    // Closing the app marks it disconnected but known.
    await client.setPresence({ ...presence(tools.url, tools.headers), open: false });
    const after = (await client.listConnectors()).find((c) => c.app === "demo");
    expect(after?.connected).toBe(false);
  });
});

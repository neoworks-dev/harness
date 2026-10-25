import { afterEach, describe, expect, test } from "bun:test";
import type { Envelope } from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import { FocusManager } from "../src/focus.ts";
import { ConnectorRegistry } from "../src/gateway/registry.ts";
import { type ClientHandle, Hub } from "../src/hub.ts";
import { silentLogger } from "../src/log.ts";
import { SessionManager } from "../src/sessions.ts";
import { SessionStore } from "../src/store.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { ViewManager } from "../src/view/views.ts";
import { FakeAgent } from "./fake-agent.ts";

interface Setup {
  store: SessionStore;
  agent: FakeAgent;
  sessions: SessionManager;
  focus: FocusManager;
  hub: Hub;
}

interface TestClient {
  received: Envelope[];
  send(envelope: Envelope | Record<string, unknown>): void;
  handle: ClientHandle;
  /** The ids of the session.removed broadcasts it got. */
  removed(): string[];
}

/** A hub with a focus and views, like the daemon's. */
function setup(): Setup {
  const store = new SessionStore(":memory:");
  const agent = new FakeAgent();
  const registry = new ConnectorRegistry(
    store,
    () => Promise.reject(new Error("no MCP in these tests")),
    silentLogger,
  );
  const sessions = new SessionManager(store, agent, { defaultCwd: "/", log: silentLogger });
  const focus = new FocusManager(store, sessions, registry, silentLogger);
  const views = new ViewManager(store, () => undefined, silentLogger);
  const hub = new Hub(store, sessions, { version: "t", pid: 1, startedAt: 0 }, silentLogger, {
    connectors: registry,
    focus,
    views,
  });
  return { store, agent, sessions, focus, hub };
}

/** A client that said hello, whose received frames are checked against the schema. */
function client(hub: Hub): TestClient {
  const received: Envelope[] = [];
  const handle = hub.connect({
    send(frame) {
      const result = validateEnvelope(JSON.parse(frame));
      if (!result.ok) throw new Error(`daemon sent an invalid frame: ${result.error}\n${frame}`);
      received.push(result.value);
    },
  });
  const send = (envelope: Envelope | Record<string, unknown>): void =>
    handle.receive(JSON.stringify(envelope));
  send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
  const removed = (): string[] => {
    const ids: string[] = [];
    for (const envelope of received) {
      if (envelope.topic === "session.removed") ids.push(envelope.payload.id);
    }
    return ids;
  };
  return { received, send, handle, removed };
}

function subscribe(c: TestClient, session: string): void {
  c.send({ topic: "session.subscribe", id: "s", payload: { session } });
}

function unsubscribe(c: TestClient, session: string): void {
  c.send({ topic: "session.unsubscribe", id: "u", payload: { session } });
}

describe("an empty session", () => {
  test("is deleted when its last subscriber unsubscribes, and every client is told", () => {
    const { hub, sessions, store } = setup();
    const id = sessions.create({}).id;
    const follower = client(hub);
    const bystander = client(hub);
    subscribe(follower, id);

    unsubscribe(follower, id);

    expect(sessions.info(id)).toBeUndefined();
    expect(store.read(id)).toEqual([]);
    expect(sessions.list().map((info) => info.id)).not.toContain(id);
    expect(follower.removed()).toEqual([id]);
    expect(bystander.removed()).toEqual([id]);
  });

  test("is deleted when its last follower disconnects", () => {
    const { hub, sessions } = setup();
    const id = sessions.create({}).id;
    const follower = client(hub);
    const bystander = client(hub);
    subscribe(follower, id);
    follower.send({ topic: "view.subscribe", id: "v", payload: { session: id } });

    follower.handle.closed();

    expect(sessions.info(id)).toBeUndefined();
    expect(bystander.removed()).toEqual([id]);
  });

  test("is deleted when the last view of it goes", () => {
    const { hub, sessions } = setup();
    const id = sessions.create({}).id;
    const c = client(hub);
    c.send({ topic: "view.subscribe", id: "v", payload: { session: id } });

    c.send({ topic: "view.unsubscribe", id: "w", payload: { session: id } });

    expect(sessions.info(id)).toBeUndefined();
    expect(c.removed()).toEqual([id]);
  });

  test("stays while another client still follows it, by subscription or by view", () => {
    const { hub, sessions } = setup();
    const id = sessions.create({}).id;
    const a = client(hub);
    const b = client(hub);
    const c = client(hub);
    subscribe(a, id);
    subscribe(b, id);
    c.send({ topic: "view.subscribe", id: "v", payload: { session: id } });

    unsubscribe(a, id);
    b.handle.closed();

    expect(sessions.info(id)).toBeDefined();
    expect(c.removed()).toEqual([]);
    // Following it again and letting go once more still deletes it.
    c.send({ topic: "view.unsubscribe", id: "w", payload: { session: id } });
    expect(sessions.info(id)).toBeUndefined();
  });

  test("stays while it is the agent's current session, and goes once the focus leaves", () => {
    const { hub, sessions, focus } = setup();
    const id = sessions.create({}).id;
    focus.move({ session: id }, "user");
    const c = client(hub);
    subscribe(c, id);

    unsubscribe(c, id);
    expect(sessions.info(id)).toBeDefined();

    const other = sessions.create({}).id;
    focus.move({ session: other }, "user");
    expect(sessions.info(id)).toBeUndefined();
    expect(c.removed()).toEqual([id]);
  });

  test("followed by a client stays when the focus leaves it", () => {
    const { hub, sessions, focus } = setup();
    const id = sessions.create({}).id;
    const c = client(hub);
    subscribe(c, id);
    focus.move({ session: id }, "user");

    focus.move({ session: sessions.create({}).id }, "user");

    expect(sessions.info(id)).toBeDefined();
    unsubscribe(c, id);
    expect(sessions.info(id)).toBeUndefined();
  });

  test("stays while its first turn is queued, before the user message is logged", async () => {
    const { hub, sessions, agent } = setup();
    const id = sessions.create({}).id;
    const c = client(hub);
    subscribe(c, id);
    sessions.input(id, "cli", "hello");

    unsubscribe(c, id);

    expect(sessions.info(id)).toBeDefined();
    await agent.started(1);
    agent.release();
    await sessions.idle(id);
    expect(sessions.info(id)).toBeDefined();
  });
});

describe("a session that is not empty", () => {
  test("survives losing its followers once the user wrote in it", () => {
    const { hub, sessions, store } = setup();
    const id = sessions.create({}).id;
    store.append(id, { type: "user_message", app: "cli", text: "hi" });
    const c = client(hub);
    subscribe(c, id);

    unsubscribe(c, id);
    c.handle.closed();

    expect(sessions.info(id)).toBeDefined();
    expect(store.read(id)).toHaveLength(1);
  });

  test("a parent and its child both survive, empty as they are", () => {
    const { hub, sessions } = setup();
    const parent = sessions.create({}).id;
    const child = sessions.create({ parent }).id;
    const c = client(hub);
    subscribe(c, parent);
    subscribe(c, child);

    unsubscribe(c, child);
    unsubscribe(c, parent);

    expect(sessions.info(parent)).toBeDefined();
    expect(sessions.info(child)).toBeDefined();
    expect(c.removed()).toEqual([]);
  });
});

describe("SessionManager.remove", () => {
  test("closes the agent session it loaded and tells onRemove listeners", async () => {
    const { sessions, agent } = setup();
    const id = sessions.create({}).id;
    // Picking a model before the first message loads the harness's agent session.
    await sessions.configOptions(id);
    expect(agent.created).toHaveLength(1);
    const removed: string[] = [];
    sessions.onRemove((session) => removed.push(session));

    expect(sessions.removable(id)).toBe(true);
    sessions.remove(id);
    await Bun.sleep(0);

    expect(removed).toEqual([id]);
    expect(agent.closed).toEqual(["other"]);
    expect(() => sessions.input(id, "cli", "late")).toThrow(`no session ${id}`);
  });
});

describe("at daemon start", () => {
  const daemons: TestDaemon[] = [];
  afterEach(async () => {
    for (const daemon of daemons.splice(0)) await daemon.stop();
  });

  test("empty sessions are pruned; written-in ones, parents and children are kept", async () => {
    const first = await startTestDaemon();
    const empty = first.sessions.create({}).id;
    const written = first.sessions.create({}).id;
    first.store.append(written, { type: "user_message", app: "cli", text: "hi" });
    const parent = first.sessions.create({}).id;
    const child = first.sessions.create({ parent }).id;
    first.focus.move({ session: empty }, "user");
    await first.stop();

    const again = await startTestDaemon({ dir: first.dir });
    daemons.push(again);
    const ids = again.sessions.list().map((info) => info.id);

    expect(ids).not.toContain(empty);
    expect(ids).toContain(written);
    expect(ids).toContain(parent);
    expect(ids).toContain(child);
    // The saved focus was pruned: it falls back to a session that is left.
    expect(ids).toContain(again.focus.current().session);
  });

  test("with every session pruned, the focus starts a fresh one", async () => {
    const first = await startTestDaemon();
    const current = first.focus.current().session;
    await first.stop();

    const again = await startTestDaemon({ dir: first.dir });
    daemons.push(again);
    const ids = again.sessions.list().map((info) => info.id);

    expect(ids).not.toContain(current);
    expect(ids).toEqual([again.focus.current().session]);
  });
});

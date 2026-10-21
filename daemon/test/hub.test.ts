import { describe, expect, test } from "bun:test";
import type { Envelope, SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import { type ClientHandle, Hub } from "../src/hub.ts";
import { silentLogger } from "../src/log.ts";
import { SessionManager } from "../src/sessions.ts";
import { SessionStore } from "../src/store.ts";
import { FakeAgent } from "./fake-agent.ts";

interface Setup {
  store: SessionStore;
  agent: FakeAgent;
  sessions: SessionManager;
  hub: Hub;
}

interface TestClient {
  received: Envelope[];
  send(envelope: Envelope | Record<string, unknown>): void;
  handle: ClientHandle;
  events(session?: string): SessionEventRecord[];
  last(): Envelope | undefined;
}

function setup(): Setup {
  const store = new SessionStore(":memory:");
  const agent = new FakeAgent();
  const sessions = new SessionManager(store, agent, { defaultCwd: "/home", log: silentLogger });
  const hub = new Hub(store, sessions, { version: "test", pid: 1, startedAt: 0 }, silentLogger);
  return { store, agent, sessions, hub };
}

/** A client whose received frames are checked against the schema. */
function client(hub: Hub, hello = true): TestClient {
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
  if (hello) send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
  const events = (session?: string): SessionEventRecord[] => {
    const out: SessionEventRecord[] = [];
    for (const e of received) {
      if (e.topic === "session.events" && (!session || e.payload.session === session)) {
        out.push(e.payload);
      }
    }
    return out;
  };
  const last = (): Envelope | undefined => received.at(-1);
  return { received, send, handle, events, last };
}

describe("hub", () => {
  test("hello → welcome", () => {
    const { hub } = setup();
    const c = client(hub);
    expect(c.received).toEqual([
      {
        topic: "bus.welcome",
        payload: { protocol: 1, daemonVersion: "test", pid: 1, startedAt: 0 },
      },
    ]);
  });

  test("hello with another protocol version is rejected", () => {
    const { hub } = setup();
    const c = client(hub, false);
    c.send({ topic: "bus.hello", payload: { protocol: 2, client: "future" } });
    expect(c.last()).toMatchObject({
      topic: "bus.error",
      payload: { code: "unsupported_protocol" },
    });
    c.send({ topic: "session.list", id: "1" });
    expect(c.last()).toMatchObject({
      topic: "bus.error",
      id: "1",
      payload: { code: "bad_request" },
    });
  });

  test("requests before hello are rejected", () => {
    const { hub } = setup();
    const c = client(hub, false);
    c.send({ topic: "session.list", id: "1" });
    expect(c.last()).toMatchObject({
      topic: "bus.error",
      id: "1",
      payload: { code: "bad_request" },
    });
  });

  test("invalid frames get bus.error, with the id when there is one", () => {
    const { hub } = setup();
    const c = client(hub);
    c.handle.receive("not json");
    expect(c.last()).toMatchObject({ topic: "bus.error", payload: { code: "bad_request" } });
    c.send({ topic: "session.input", id: "7", payload: { session: "s" } });
    expect(c.last()).toMatchObject({ topic: "bus.error", id: "7" });
    c.send({
      topic: "session.events",
      payload: { session: "s", seq: 1, ts: 0, event: { type: "app_disconnected", app: "x" } },
    });
    expect(c.last()).toMatchObject({ topic: "bus.error", payload: { code: "bad_request" } });
  });

  test("create, list, and not_found errors", () => {
    const { hub } = setup();
    const c = client(hub);
    c.send({ topic: "session.create", id: "1", payload: { cwd: "/w", title: "T" } });
    const created = c.last();
    if (created?.topic !== "session.info") throw new Error("expected session.info");
    expect(created).toMatchObject({
      id: "1",
      payload: { cwd: "/w", title: "T", lastSeq: 0, state: "idle" },
    });

    c.send({ topic: "session.list", id: "2" });
    expect(c.last()).toMatchObject({
      topic: "session.list_result",
      id: "2",
      payload: { sessions: [{ id: created.payload.id }] },
    });

    for (const topic of ["session.subscribe", "session.cancel"]) {
      c.send({ topic, id: "3", payload: { session: "s_nope" } });
      expect(c.last()).toMatchObject({
        topic: "bus.error",
        id: "3",
        payload: { code: "not_found" },
      });
    }
    c.send({
      topic: "session.input",
      id: "4",
      payload: { session: "s_nope", app: "cli", text: "x" },
    });
    expect(c.last()).toMatchObject({ topic: "bus.error", id: "4", payload: { code: "not_found" } });
  });

  test("input is acked, runs, and streams to every subscriber", async () => {
    const { hub, agent, sessions } = setup();
    const a = client(hub);
    const b = client(hub);
    const id = sessions.create({}).id;
    a.send({ topic: "session.subscribe", id: "s", payload: { session: id } });
    b.send({ topic: "session.subscribe", id: "s", payload: { session: id } });

    b.send({
      topic: "session.input",
      id: "in",
      payload: { session: id, app: "overlay", text: "hi" },
    });
    expect(b.last()).toEqual({ topic: "bus.ack", id: "in" });
    await agent.started(1);
    agent.release();
    await sessions.idle(id);

    const expected = ["user_message", "agent_message_chunk", "turn_ended"];
    expect(a.events(id).map((r): string => r.event.type)).toEqual(expected);
    expect(b.events(id)).toEqual(a.events(id));
    expect(a.events(id)[0]?.event).toEqual({ type: "user_message", app: "overlay", text: "hi" });
  });

  test("subscribe replays from fromSeq, then marks the switch to live", () => {
    const { hub, store, sessions } = setup();
    const id = sessions.create({}).id;
    for (let i = 0; i < 5; i++) store.append(id, { type: "app_disconnected", app: `a${i}` });
    const c = client(hub);
    c.send({ topic: "session.subscribe", id: "s", payload: { session: id, fromSeq: 3 } });
    expect(c.events(id).map((r) => r.seq)).toEqual([3, 4, 5]);
    expect(c.last()).toMatchObject({
      topic: "session.subscribed",
      id: "s",
      payload: { replayedThrough: 5 },
    });
    store.append(id, { type: "app_disconnected", app: "live" });
    expect(c.events(id).map((r) => r.seq)).toEqual([3, 4, 5, 6]);
  });

  test("replay-then-live has no gaps or duplicates while events keep arriving", async () => {
    const { hub, store, sessions } = setup();
    const id = sessions.create({}).id;
    const clients: TestClient[] = [];
    const fromSeqs: number[] = [];
    // Interleave appends with subscribes at varying points and starting seqs.
    for (let i = 1; i <= 300; i++) {
      store.append(id, { type: "agent_message_chunk", messageId: "m", text: String(i) });
      if (i % 37 === 0) {
        const c = client(hub);
        const fromSeq = (i % 3) + 1 + Math.floor(i / 2);
        c.send({ topic: "session.subscribe", id: "s", payload: { session: id, fromSeq } });
        clients.push(c);
        fromSeqs.push(fromSeq);
      }
      if (i % 11 === 0) await Bun.sleep(0);
    }
    const last = store.getSession(id)?.lastSeq ?? 0;
    clients.forEach((c, n) => {
      const from = fromSeqs[n] ?? 1;
      const seqs = c.events(id).map((r) => r.seq);
      expect(seqs).toEqual(Array.from({ length: last - from + 1 }, (_, k) => from + k));
      // The marker sits exactly between replay and live.
      const marker = c.received.findIndex((e) => e.topic === "session.subscribed");
      const replayed = c.received.slice(0, marker).filter((e) => e.topic === "session.events");
      const through = c.received[marker];
      if (through?.topic !== "session.subscribed") throw new Error("missing marker");
      expect(replayed.length).toBe(Math.max(0, through.payload.replayedThrough - from + 1));
    });
  });

  test("unsubscribe and disconnect stop delivery", () => {
    const { hub, store, sessions } = setup();
    const id = sessions.create({}).id;
    const a = client(hub);
    const b = client(hub);
    for (const c of [a, b])
      c.send({ topic: "session.subscribe", id: "s", payload: { session: id } });
    a.send({ topic: "session.unsubscribe", id: "u", payload: { session: id } });
    expect(a.last()).toEqual({ topic: "bus.ack", id: "u" });
    b.handle.closed();
    store.append(id, { type: "app_disconnected", app: "x" });
    expect(a.events(id)).toEqual([]);
    expect(b.events(id)).toEqual([]);
    expect(hub.clientCount).toBe(1);
  });

  test("cancel is acked and cancels the running turn", async () => {
    const { hub, agent, sessions, store } = setup();
    const id = sessions.create({}).id;
    const c = client(hub);
    c.send({ topic: "session.input", payload: { session: id, app: "cli", text: "x" } });
    await agent.started(1);
    c.send({ topic: "session.cancel", id: "c", payload: { session: id } });
    expect(c.last()).toEqual({ topic: "bus.ack", id: "c" });
    await sessions.idle(id);
    expect(store.read(id).at(-1)?.event).toEqual({ type: "turn_ended", stopReason: "cancelled" });
  });

  test("a client whose transport throws doesn't break others", () => {
    const { hub, store, sessions } = setup();
    const id = sessions.create({}).id;
    const ok = client(hub);
    ok.send({ topic: "session.subscribe", id: "s", payload: { session: id } });
    const bad = hub.connect({
      send() {
        throw new Error("socket closed");
      },
    });
    bad.receive(JSON.stringify({ topic: "bus.hello", payload: { protocol: 1, client: "bad" } }));
    bad.receive(JSON.stringify({ topic: "session.subscribe", id: "s", payload: { session: id } }));
    store.append(id, { type: "app_disconnected", app: "x" });
    expect(ok.events(id)).toHaveLength(1);
  });
});

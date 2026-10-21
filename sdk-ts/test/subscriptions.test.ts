import { describe, expect, test } from "bun:test";
import type { ViewPatch, ViewRef, ViewSnapshot } from "@neoworks-dev/neo-protocol/types";
import { connect, DisconnectedError, fixedAddress, SuiteError } from "../src/index.ts";
import type { Client } from "../src/index.ts";
import { cleanupAfterEach } from "./support/cleanup.ts";
import { FakeDaemon } from "./support/fake-daemon.ts";
import { viewItem } from "./support/fixtures.ts";
import { Recorder } from "./support/recorder.ts";
import { connectTo, startDaemon, startRig } from "./support/real-daemon.ts";
import { rejection, until } from "./support/wait.ts";

const later = cleanupAfterEach();

async function fakeRig(): Promise<{ daemon: FakeDaemon; client: Client }> {
  const daemon = new FakeDaemon();
  later(() => daemon.stop());
  const client = await connect({ client: "test", address: fixedAddress(daemon) });
  later(() => client.close());
  return { daemon, client };
}

/** What the fake daemon was asked to follow or stop following, in order. */
function viewRequests(daemon: FakeDaemon, topic: "view.subscribe" | "view.unsubscribe"): ViewRef[] {
  const refs: ViewRef[] = [];
  for (const frame of daemon.received) {
    if (frame.topic === topic) refs.push(frame.payload);
  }
  return refs;
}

/** Collects what a view subscription delivers. */
function viewRecorder(): {
  snapshots: ViewSnapshot[];
  patches: ViewPatch[];
  errors: Error[];
  options: Parameters<Client["subscribeView"]>[1];
} {
  const snapshots: ViewSnapshot[] = [];
  const patches: ViewPatch[] = [];
  const errors: Error[] = [];
  const options = {
    onSnapshot: (snapshot: ViewSnapshot) => snapshots.push(snapshot),
    onPatch: (patch: ViewPatch) => patches.push(patch),
    onError: (error: Error) => errors.push(error),
  };
  return { snapshots, patches, errors, options };
}

const SET_B: ViewPatch["ops"] = [{ op: "set", item: viewItem("b", "world") }];

describe("subscribe (session events)", () => {
  test("replays the history, then follows live", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    await client.input(id, "one");
    const events = new Recorder();
    // Subscribing after (or during) the first turn replays what is there.
    const stop = client.subscribe(id, events);
    await until(() => events.live === 1 && events.turnsEnded() === 1, "replay");
    expect(events.types()[0]).toBe("user_message");
    expect(events.text()).toBe("echo #1: one");

    await client.input(id, "two");
    await until(() => events.turnsEnded() === 2, "second turn");
    expect(events.text()).toBe("echo #1: oneecho #2: two");
    expect(events.seqs).toEqual(events.seqs.map((_, index) => index + 1));
    stop();
  });

  test("fromSeq skips older events", async () => {
    const { client } = await startRig(later);
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
  });

  test("stopping a subscription stops its events", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    const events = new Recorder();
    const stop = client.subscribe(id, events);
    await until(() => events.live === 1, "live");
    stop();
    stop();
    await client.input(id, "x");
    await Bun.sleep(150);
    expect(events.records).toEqual([]);
  });

  test("two subscriptions to a session both get every event; one stopping leaves the other", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    const first = new Recorder();
    const second = new Recorder();
    const stopFirst = client.subscribe(id, first);
    client.subscribe(id, second);
    await until(() => first.live === 1 && second.live === 1, "both live");

    stopFirst();
    await client.input(id, "x");
    await until(() => second.turnsEnded() === 1, "turn");
    expect(first.records).toEqual([]);
    expect(second.seqs).toEqual(second.seqs.map((_, index) => index + 1));
  });

  test("an unknown session reaches the subscription's onError", async () => {
    const { client } = await startRig(later);
    const errors: Error[] = [];
    client.subscribe("nope", { onEvent: () => {}, onError: (error) => errors.push(error) });
    await until(() => errors.length === 1, "error");
    expect(errors[0]).toBeInstanceOf(SuiteError);
    expect(errors[0]).toMatchObject({ code: "not_found" });
  });

  test("without an onError of its own, a refusal goes to the client's onError", async () => {
    const daemon = await startDaemon(later);
    const errors: Error[] = [];
    const client = await connectTo(later, daemon, { onError: (error) => errors.push(error) });
    client.subscribe("nope", { onEvent: () => {} });
    await until(() => errors.length === 1, "error");
    expect(errors[0]).toMatchObject({ code: "not_found" });
  });

  test("a throwing onEvent goes to the client's onError and does not stop later events", async () => {
    const daemon = await startDaemon(later);
    const errors: Error[] = [];
    const client = await connectTo(later, daemon, { onError: (error) => errors.push(error) });
    const { id } = await client.createSession();
    const seen: number[] = [];
    client.subscribe(id, {
      onEvent: (_event, record) => {
        seen.push(record.seq);
        throw new Error("bad handler");
      },
    });
    await client.input(id, "x");
    await until(() => seen.length >= 3, "events");
    expect(errors.length).toBe(seen.length);
    expect(seen).toEqual(seen.map((_, index) => index + 1));
  });

  test("after a daemon restart it resumes without gaps or duplicates", async () => {
    const first = await startDaemon(later);
    const client = await connectTo(later, first);
    const { id } = await client.createSession();
    const events = new Recorder();
    client.subscribe(id, events);
    await client.input(id, "before");
    await until(() => events.turnsEnded() === 1, "first turn");

    // A turn is running when the daemon stops.
    await client.input(id, "wait:10000 interrupted");
    await until(
      () => events.types().filter((type) => type === "user_message").length === 2,
      "start",
    );
    await first.stop();
    await client.state.waitFor(2000, (state) => state === "reconnecting");

    await startDaemon(later, first.dir);
    await client.state.waitFor(8000, (state) => state === "open");
    await until(() => events.live === 2, "live again");

    await client.input(id, "after");
    await until(() => events.turnsEnded() === 3, "turns after the restart");
    expect(events.seqs).toEqual(events.seqs.map((_, index) => index + 1));
    expect(events.text()).toContain("echo #3: after");
    // The interrupted turn ended one way or the other, and is in the replay.
    expect(["cancelled", "error"]).toContain(events.turnEnds()[1] ?? "");
  });
});

describe("subscribeView", () => {
  test("a snapshot first, then patches; stopping sends view.unsubscribe", async () => {
    const { daemon, client } = await fakeRig();
    daemon.items = [viewItem("a", "hello")];
    const view = viewRecorder();
    const stop = client.subscribeView("s1", view.options);
    await until(() => view.snapshots.length === 1, "snapshot");
    expect(view.snapshots[0]).toEqual({ session: "s1", seq: 1, items: [viewItem("a", "hello")] });

    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 2, ops: SET_B } });
    // Another session's patches are not ours.
    daemon.broadcast({ topic: "view.patch", payload: { session: "other", seq: 9, ops: SET_B } });
    await until(() => view.patches.length === 1, "patch");
    expect(view.patches).toEqual([{ session: "s1", seq: 2, ops: SET_B }]);

    stop();
    await until(() => daemon.topics().includes("view.unsubscribe"), "unsubscribe");
    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 3, ops: SET_B } });
    await Bun.sleep(50);
    expect(view.patches).toHaveLength(1);
  });

  test("patches that arrive before the snapshot are not delivered", async () => {
    const { daemon, client } = await fakeRig();
    const view = viewRecorder();
    // A patch right in front of the snapshot would apply to a state that doesn't exist yet.
    client.subscribeView("s1", view.options);
    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 1, ops: SET_B } });
    await until(() => view.snapshots.length === 1, "snapshot");
    expect(view.patches).toEqual([]);
  });

  test("a refused subscription reports a SuiteError through onError", async () => {
    const { client } = await fakeRig();
    const view = viewRecorder();
    client.subscribeView("missing", view.options);
    await until(() => view.errors.length === 1, "error");
    expect(view.errors[0]).toBeInstanceOf(SuiteError);
    expect(view.errors[0]).toMatchObject({ code: "not_found" });
  });

  test("a reconnect subscribes again and delivers a fresh snapshot", async () => {
    const { daemon, client } = await fakeRig();
    daemon.items = [viewItem("a", "one")];
    const view = viewRecorder();
    client.subscribeView("s1", view.options);
    await until(() => view.snapshots.length === 1, "first snapshot");

    daemon.items = [viewItem("a", "one"), viewItem("b", "two")];
    daemon.seq = 7;
    daemon.dropConnections();
    await until(() => view.snapshots.length === 2, "snapshot after reconnect", 8000);
    expect(view.snapshots[1]?.seq).toBe(7);
    expect(view.snapshots[1]?.items).toHaveLength(2);
    expect(viewRequests(daemon, "view.subscribe")).toHaveLength(2);
  });

  test("subscribing while the connection is down starts when it is back", async () => {
    const { daemon, client } = await fakeRig();
    daemon.items = [viewItem("a", "early")];
    daemon.dropConnections();
    await client.state.waitFor(2000, (state) => state === "reconnecting");

    const view = viewRecorder();
    client.subscribeView("s1", view.options);
    expect(viewRequests(daemon, "view.subscribe")).toEqual([]);
    await until(() => view.snapshots.length === 1, "snapshot", 8000);
    expect(view.errors).toEqual([]);
  });

  test("a throwing onSnapshot goes to the client's onError", async () => {
    const daemon = new FakeDaemon();
    later(() => daemon.stop());
    const errors: Error[] = [];
    const client = await connect({
      client: "test",
      address: fixedAddress(daemon),
      onError: (error) => errors.push(error),
    });
    later(() => client.close());
    client.subscribeView("s1", {
      onSnapshot: () => {
        throw new Error("bad snapshot handler");
      },
      onPatch: () => {},
    });
    await until(() => errors.length === 1, "error");
    expect(errors[0]?.message).toBe("bad snapshot handler");
  });
});

describe("agent views", () => {
  test("have their own snapshot and patches, apart from the session's own view", async () => {
    const { daemon, client } = await fakeRig();
    daemon.items = [viewItem("main", "the session")];
    daemon.agentItems = { claude: [viewItem("work", "the agent")] };
    const own = viewRecorder();
    const claude = viewRecorder();
    client.subscribeView("s1", own.options);
    client.subscribeView("s1", { ...claude.options, agent: "claude" });
    await until(() => own.snapshots.length === 1 && claude.snapshots.length === 1, "snapshots");

    expect(own.snapshots[0]).toEqual({ session: "s1", seq: 1, items: daemon.items });
    expect(claude.snapshots[0]).toEqual({
      session: "s1",
      agent: "claude",
      seq: 1,
      items: [viewItem("work", "the agent")],
    });
    expect(viewRequests(daemon, "view.subscribe")).toEqual([
      { session: "s1" },
      { session: "s1", agent: "claude" },
    ]);

    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 2, ops: SET_B } });
    daemon.broadcast({
      topic: "view.patch",
      payload: { session: "s1", agent: "claude", seq: 3, ops: SET_B },
    });
    daemon.broadcast({
      topic: "view.patch",
      payload: { session: "s1", agent: "codex", seq: 4, ops: SET_B },
    });
    await until(() => own.patches.length === 1 && claude.patches.length === 1, "patches");
    await Bun.sleep(50);
    expect(own.patches.map((patch) => patch.seq)).toEqual([2]);
    expect(claude.patches.map((patch) => patch.seq)).toEqual([3]);
  });

  test("are stopped one at a time, and only when the last subscriber of one goes", async () => {
    const { daemon, client } = await fakeRig();
    const stopOwn = client.subscribeView("s1", viewRecorder().options);
    const stopClaude = client.subscribeView("s1", { ...viewRecorder().options, agent: "claude" });
    const stopClaudeAgain = client.subscribeView("s1", {
      ...viewRecorder().options,
      agent: "claude",
    });
    await until(() => viewRequests(daemon, "view.subscribe").length === 3, "subscriptions");

    stopClaude();
    await Bun.sleep(50);
    expect(viewRequests(daemon, "view.unsubscribe")).toEqual([]);

    stopClaudeAgain();
    await until(() => viewRequests(daemon, "view.unsubscribe").length === 1, "unsubscribe");
    expect(viewRequests(daemon, "view.unsubscribe")).toEqual([{ session: "s1", agent: "claude" }]);

    stopOwn();
    await until(() => viewRequests(daemon, "view.unsubscribe").length === 2, "unsubscribe");
    expect(viewRequests(daemon, "view.unsubscribe")[1]).toEqual({ session: "s1" });
  });

  test("are subscribed again after a reconnect, each with a fresh snapshot", async () => {
    const { daemon, client } = await fakeRig();
    daemon.agentItems = { claude: [viewItem("work", "before")] };
    const own = viewRecorder();
    const claude = viewRecorder();
    client.subscribeView("s1", own.options);
    client.subscribeView("s1", { ...claude.options, agent: "claude" });
    await until(() => own.snapshots.length === 1 && claude.snapshots.length === 1, "snapshots");

    daemon.agentItems = { claude: [viewItem("work", "after")] };
    daemon.seq = 5;
    daemon.dropConnections();
    await until(() => own.snapshots.length === 2 && claude.snapshots.length === 2, "again", 8000);
    expect(claude.snapshots[1]).toMatchObject({ agent: "claude", seq: 5 });
    expect(claude.snapshots[1]?.items).toEqual([viewItem("work", "after")]);
    expect(own.snapshots[1]?.agent).toBeUndefined();
  });

  test("a real daemon answers an agent's view with that agent's own, empty here", async () => {
    const { client } = await startRig(later);
    const { id } = await client.createSession();
    await client.input(id, "hello");
    const own = viewRecorder();
    const claude = viewRecorder();
    client.subscribeView(id, own.options);
    client.subscribeView(id, { ...claude.options, agent: "claude" });
    await until(() => own.snapshots.length === 1 && claude.snapshots.length === 1, "snapshots");

    expect(own.snapshots[0]?.agent).toBeUndefined();
    expect(claude.snapshots[0]?.agent).toBe("claude");
    expect(claude.snapshots[0]?.items).toEqual([]);
    await until(() => own.patches.length > 0 || (own.snapshots[0]?.items.length ?? 0) > 0, "items");
    expect(claude.patches).toEqual([]);
  });
});

describe("after the client is closed", () => {
  test("subscribing does nothing and calls reject", async () => {
    const { daemon, client } = await fakeRig();
    await client.close();
    const view = viewRecorder();
    client.subscribeView("s1", view.options);
    await Bun.sleep(50);
    expect(viewRequests(daemon, "view.subscribe")).toEqual([]);
    expect(await rejection(client.cancel("s1"))).toBeInstanceOf(DisconnectedError);
  });
});

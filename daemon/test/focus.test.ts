import { describe, expect, test } from "bun:test";
import type { AgentFocus, AppPresence, Envelope, SessionEvent } from "@suite/protocol/types";
import { BadRequestError, FocusManager } from "../src/focus.ts";
import { Gateway } from "../src/gateway/gateway.ts";
import { ConnectorRegistry } from "../src/gateway/registry.ts";
import { Hub } from "../src/hub.ts";
import { silentLogger } from "../src/log.ts";
import { NotFoundError, SessionManager } from "../src/sessions.ts";
import { SessionStore } from "../src/store.ts";
import { FakeAgent } from "./fake-agent.ts";

interface Setup {
  store: SessionStore;
  agent: FakeAgent;
  sessions: SessionManager;
  registry: ConnectorRegistry;
  focus: FocusManager;
  changes: AgentFocus[];
}

function setup(store = new SessionStore(":memory:")): Setup {
  const agent = new FakeAgent();
  const registry = new ConnectorRegistry(
    store,
    () => Promise.reject(new Error("no MCP in these tests")),
    silentLogger,
  );
  let focus: FocusManager | undefined;
  const sessions = new SessionManager(store, agent, {
    defaultCwd: "/",
    log: silentLogger,
    connectors: () => registry.snapshot(),
    focus: (id) => focus?.focusOf(id),
  });
  focus = new FocusManager(store, sessions, registry, silentLogger);
  const changes: AgentFocus[] = [];
  focus.onChange((f) => changes.push(f));
  return { store, agent, sessions, registry, focus, changes };
}

const app = (name: string, extra: Partial<AppPresence> = {}): AppPresence => ({
  app: name,
  kind: "app",
  open: true,
  ...extra,
});

describe("current session", () => {
  test("created at startup when there is none, and kept across restarts", () => {
    const store = new SessionStore(":memory:");
    const first = setup(store);
    const session = first.focus.current().session;
    expect(first.sessions.list().map((s) => s.id)).toEqual([session]);
    const other = first.sessions.create({}).id;
    first.focus.move({ session: other }, "user");

    const second = setup(store);
    expect(second.focus.current()).toEqual({ session: other, reason: "initial" });
    expect(second.sessions.list()).toHaveLength(2);
  });

  test("a user switching sessions goes home and reports reason session", async () => {
    const { focus, registry, sessions, changes } = setup();
    await registry.presence({}, app("overlay", { home: true }));
    await registry.presence({}, app("pad"));
    focus.move({ app: "pad" }, "user");
    const fresh = sessions.create({}).id;
    focus.move({ session: fresh }, "user");
    expect(changes.at(-1)).toEqual({
      session: fresh,
      app: "overlay",
      from: "pad",
      reason: "session",
    });
  });
});

describe("agent focus", () => {
  test("home takes the agent, moves go to apps, closing falls back home", async () => {
    const { focus, registry, changes } = setup();
    const session = focus.current().session;
    expect(focus.current()).toEqual({ session, reason: "initial" }); // nothing open yet

    const overlay = {};
    const pad = {};
    await registry.presence(overlay, app("overlay", { home: true }));
    await registry.presence(pad, app("pad"));
    focus.move({ app: "pad" }, "user");
    await registry.disconnect(pad);
    await registry.disconnect(overlay);
    await registry.presence(overlay, app("overlay", { home: true }));

    expect(changes).toEqual([
      { session, app: "overlay", reason: "opened" },
      { session, app: "pad", from: "overlay", reason: "user" },
      { session, app: "overlay", from: "pad", reason: "closed" },
      { session, from: "overlay", reason: "closed" },
      { session, app: "overlay", reason: "opened" },
    ]);
  });

  test("moving where it already is changes nothing; no app means home", async () => {
    const { focus, registry, changes } = setup();
    await registry.presence({}, app("overlay", { home: true }));
    await registry.presence({}, app("pad"));
    focus.move({ app: "pad" }, "user");
    focus.move({ app: "pad" }, "user");
    focus.move({}, "user");
    expect(changes.map((c) => c.app)).toEqual(["overlay", "pad", "overlay"]);
  });

  test("impossible moves throw", async () => {
    const { focus, registry } = setup();
    await registry.presence({}, { app: "lsp", kind: "service", open: true });
    expect(() => focus.move({ app: "pad" }, "user")).toThrow(NotFoundError);
    expect(() => focus.move({ app: "lsp" }, "user")).toThrow(BadRequestError);
    expect(() => focus.move({ session: "s_nope" }, "user")).toThrow(NotFoundError);
  });
});

describe("focus in the transcript", () => {
  test("the current session's turns start with <focus> when app or context changed", async () => {
    const { focus, registry, sessions, agent, store } = setup();
    const id = focus.current().session;
    const turn = async (n: number): Promise<SessionEvent[]> => {
      sessions.input(id, "cli", `turn ${n}`);
      await agent.started(n + 1);
      agent.release();
      await sessions.idle(id);
      return (agent.prompts[n]?.pendingEvents ?? []).filter((e) => e.type === "focus_changed");
    };
    const pad = {};
    await registry.presence({}, app("overlay", { home: true }));
    expect(await turn(0)).toEqual([{ type: "focus_changed", app: "overlay" }]);
    expect(await turn(1)).toEqual([]);

    await registry.presence(pad, app("pad", { context: { file: "a.txt" } }));
    focus.move({ app: "pad" }, "user");
    expect(await turn(2)).toEqual([
      { type: "focus_changed", app: "pad", context: { file: "a.txt" } },
    ]);
    await registry.presence(pad, app("pad", { context: { file: "b.txt" } }));
    expect(await turn(3)).toEqual([
      { type: "focus_changed", app: "pad", context: { file: "b.txt" } },
    ]);

    // Another session isn't where the agent is: no focus there.
    const other = sessions.create({}).id;
    sessions.input(other, "cli", "elsewhere");
    await agent.started(5);
    agent.release();
    await sessions.idle(other);
    expect(store.readTypes(other, ["focus_changed"])).toEqual([]);
  });
});

describe("over the bus and the gateway", () => {
  function hubSetup(): Setup & { hub: Hub; gateway: Gateway } {
    const s = setup();
    const gateway = new Gateway(s.registry, {
      log: silentLogger,
      focus: (target, session) => {
        try {
          s.focus.move({ app: target, session }, "agent");
          return undefined;
        } catch (err) {
          return String(err);
        }
      },
    });
    const hub = new Hub(s.store, s.sessions, { version: "t", pid: 1, startedAt: 0 }, silentLogger, {
      connectors: s.registry,
      focus: s.focus,
    });
    return { ...s, hub, gateway };
  }

  function client(hub: Hub): { received: Envelope[]; send: (e: unknown) => void } {
    const received: Envelope[] = [];
    const handle = hub.connect({ send: (f) => received.push(JSON.parse(f)) });
    const send = (e: unknown): void => handle.receive(JSON.stringify(e));
    send({ topic: "bus.hello", payload: { protocol: 1, client: "t" } });
    return { received, send };
  }

  test("welcome is followed by the focus; moves are broadcast, then acked", async () => {
    const { hub, registry, focus } = hubSetup();
    await registry.presence({}, app("pad"));
    const a = client(hub);
    const b = client(hub);
    expect(a.received.map((e) => e.topic)).toEqual(["bus.welcome", "agent.focus"]);
    a.send({ topic: "agent.move", id: "m", payload: { app: "pad" } });
    expect(a.received.slice(-2)).toEqual([
      {
        topic: "agent.focus",
        payload: { session: focus.current().session, app: "pad", reason: "user" },
      },
      { topic: "bus.ack", id: "m" },
    ]);
    expect(b.received.at(-1)?.topic).toBe("agent.focus");
    a.send({ topic: "agent.move", id: "x", payload: { app: "nope" } });
    expect(a.received.at(-1)).toMatchObject({
      topic: "bus.error",
      id: "x",
      payload: { code: "not_found" },
    });
  });

  test("typing in an open app's pane moves the agent there before the turn", async () => {
    const { hub, registry, focus, agent, sessions } = hubSetup();
    await registry.presence({}, app("overlay", { home: true }));
    await registry.presence({}, app("photo-editor", { context: { image: "logo.png" } }));
    await registry.presence({}, { app: "lsp", kind: "service", open: true });
    const a = client(hub);
    const session = focus.current().session;
    const input = (from: string, id: string): void =>
      a.send({ topic: "session.input", id, payload: { session, app: from, text: from } });

    input("cli", "1"); // not an app: stays home
    await agent.started(1);
    agent.release();
    await sessions.idle(session);
    expect(focus.current().app).toBe("overlay");

    input("photo-editor", "2");
    expect(
      a.received.find((e) => e.topic === "agent.focus" && e.payload.reason === "input"),
    ).toEqual({
      topic: "agent.focus",
      payload: { session, app: "photo-editor", from: "overlay", reason: "input" },
    });
    await agent.started(2);
    expect(agent.prompts[1]?.pendingEvents).toContainEqual({
      type: "focus_changed",
      app: "photo-editor",
      context: { image: "logo.png" },
    });
    agent.release();
    await sessions.idle(session);

    input("lsp", "3"); // a service isn't somewhere the agent can be
    await agent.started(3);
    agent.release();
    await sessions.idle(session);
    expect(focus.current().app).toBe("photo-editor");
  });

  test("/clear starts a fresh current session in the same directory; the agent stays put", async () => {
    const { hub, registry, focus, sessions, agent } = hubSetup();
    await registry.presence({}, app("code-editor"));
    const old = sessions.create({ cwd: "/site" }).id;
    focus.move({ app: "code-editor", session: old }, "user");
    const a = client(hub);
    a.send({
      topic: "session.input",
      id: "c",
      payload: { session: old, app: "code-editor", text: " /clear " },
    });
    const now = focus.current();
    expect(now.session).not.toBe(old);
    expect(now.app).toBe("code-editor");
    expect(sessions.info(now.session)?.cwd).toBe("/site");
    expect(sessions.info(old)).toBeDefined();
    expect(agent.prompts).toHaveLength(0); // never reaches the agent
    expect(a.received.slice(-2).map((e) => e.topic)).toEqual(["agent.focus", "bus.ack"]);
  });

  test("app.context updates what the agent sees next, without a broadcast or a store write", async () => {
    const { hub, registry, focus, agent, sessions, store } = hubSetup();
    const editor = client(hub);
    const other = client(hub);
    editor.send({
      topic: "app.presence",
      id: "p",
      payload: { app: "code-editor", kind: "app", open: true, context: { line: "1" } },
    });
    await Bun.sleep(0);
    focus.move({ app: "code-editor" }, "user");
    await Bun.sleep(0);
    const before = other.received.length;
    const saved = JSON.stringify(store.loadConnectors());

    editor.send({
      topic: "app.context",
      id: "c",
      payload: { app: "code-editor", context: { file: "/site/a.ts", line: "40" } },
    });
    expect(editor.received.at(-1)).toEqual({ topic: "bus.ack", id: "c" });
    await Bun.sleep(0);
    expect(other.received.length).toBe(before);
    expect(JSON.stringify(store.loadConnectors())).toBe(saved);
    expect(registry.list().find((c) => c.app === "code-editor")?.context).toEqual({
      file: "/site/a.ts",
      line: "40",
    });

    const session = focus.current().session;
    sessions.input(session, "cli", "hi");
    await agent.started(1);
    expect(agent.prompts[0]?.pendingEvents).toContainEqual({
      type: "focus_changed",
      app: "code-editor",
      context: { file: "/site/a.ts", line: "40" },
    });
    agent.release();
    await sessions.idle(session);

    // Only the connection that registered the app may change it.
    other.send({ topic: "app.context", id: "x", payload: { app: "code-editor", context: {} } });
    expect(other.received.at(-1)).toMatchObject({
      topic: "bus.error",
      payload: { code: "not_found" },
    });
  });

  test("connector changes are broadcast once per burst", async () => {
    const { hub, registry } = hubSetup();
    const a = client(hub);
    await registry.presence({}, app("pad", { pid: 42 }));
    await Bun.sleep(0);
    const changed = a.received.filter((e) => e.topic === "connectors.changed");
    expect(changed).toHaveLength(1);
    expect(changed[0]).toMatchObject({
      payload: { connectors: [{ app: "pad", connected: true, pid: 42 }] },
    });
  });

  test("apps_focus moves the agent of the calling session, which becomes current", async () => {
    const { gateway, registry, focus, sessions, changes } = hubSetup();
    await registry.presence({}, app("pad", { title: "Pad" }));
    const caller = sessions.create({}).id;
    const result = await gateway.call("apps_focus", { app: "pad" }, undefined, caller);
    expect(result.isError).toBeUndefined();
    expect(changes.at(-1)).toEqual({ session: caller, app: "pad", reason: "agent" });
    expect(focus.current().session).toBe(caller);

    await registry.disconnect({});
    const closed = await gateway.call("apps_focus", { app: "nope" }, undefined, caller);
    expect(closed.isError).toBe(true);
  });
});

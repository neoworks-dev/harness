import { describe, expect, test } from "bun:test";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { silentLogger } from "../src/log.ts";
import { NotFoundError, SessionManager } from "../src/sessions.ts";
import { SessionStore } from "../src/store.ts";
import { FakeAgent } from "./fake-agent.ts";

interface Setup {
  store: SessionStore;
  agent: FakeAgent;
  sessions: SessionManager;
}

/** `steer`: the user lets what they send reach a running turn at once. */
function setup(store = new SessionStore(":memory:"), steer = false): Setup {
  const agent = new FakeAgent();
  const sessions = new SessionManager(store, agent, {
    defaultCwd: "/home",
    log: silentLogger,
    steer: () => steer,
  });
  return { store, agent, sessions };
}

type UserMessage = Extract<SessionEvent, { type: "user_message" }>;

const types = (store: SessionStore, id: string): string[] =>
  store.read(id).map((r) => r.event.type);

describe("shutdown", () => {
  test("closes the agent's sessions, so their SessionEnd hooks run", async () => {
    const { agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "hi");
    await agent.started(1);
    agent.release();
    await sessions.idle(id);
    await sessions.shutdown(1000);
    expect(agent.closed).toEqual(["other"]);
  });
});

describe("turn queue", () => {
  test("a command the running turn waits for can be sent to the background", async () => {
    const { agent, sessions } = setup();
    const { id } = sessions.create({});
    expect(sessions.background(id, "t1")).toBe(false);
    sessions.input(id, "term", "build it");
    await agent.started(1);
    expect(sessions.background(id, "t1")).toBe(true);
    expect(agent.backgrounded).toEqual(["t1"]);
    agent.release();
    await sessions.idle(id);
    expect(sessions.background(id, "t1")).toBe(false);
  });

  test("unless the user lets it, what is sent waits for the turn, steerable or not", async () => {
    const { agent, sessions } = setup();
    agent.steerable = true;
    const { id } = sessions.create({});
    sessions.input(id, "term", "one");
    await agent.started(1);
    sessions.input(id, "term", "later");
    await Bun.sleep(0);
    expect(agent.steered).toEqual([]);
    expect(sessions.info(id)?.queued).toBe(1);
    agent.release();
    await agent.started(2);
    expect(agent.prompts[1]?.text).toBe("later");
    agent.release();
    await sessions.idle(id);
  });

  test("a harness that can be steered gets what is sent into the running turn at once", async () => {
    const { store, agent, sessions } = setup(undefined, true);
    agent.steerable = true;
    const { id } = sessions.create({});
    sessions.input(id, "term", "one");
    await agent.started(1);
    sessions.input(id, "term", "also this", ["/a.png"]);
    await Bun.sleep(0);
    expect(agent.steered).toEqual([{ text: "also this", images: ["/a.png"] }]);
    expect(sessions.info(id)?.queued).toBe(0);
    const users = (): string[] =>
      store
        .read(id)
        .map((r) => r.event)
        .filter((e): e is UserMessage => e.type === "user_message")
        .map((e) => e.text);
    expect(users()).toEqual(["one", "also this"]);

    // A command waits for a turn of its own; after a cancel nothing is steered into the turn.
    sessions.input(id, "term", "/claude review");
    sessions.cancel(id);
    sessions.input(id, "term", "after the cancel");
    await Bun.sleep(0);
    expect(agent.steered).toHaveLength(1);
    await agent.started(2);
    expect(agent.prompts[1]?.text).toBe("/claude review");
    agent.release();
    await agent.started(3);
    expect(agent.prompts[2]?.text).toBe("after the cancel");
    agent.release();
    await sessions.idle(id);
  });

  test("nothing is steered into a running command; it waits for the command to end", async () => {
    const { agent, sessions } = setup(undefined, true);
    agent.steerable = true;
    const { id } = sessions.create({});
    sessions.input(id, "term", "/compact");
    await agent.started(1);
    sessions.input(id, "term", "after it");
    await Bun.sleep(0);
    expect(agent.steered).toEqual([]);
    expect(sessions.info(id)?.queued).toBe(1);
    agent.release();
    await agent.started(2);
    expect(agent.prompts[1]?.text).toBe("after it");
    agent.release();
    await sessions.idle(id);
  });

  test("what waits from one app runs as one turn, up to a command", async () => {
    const { store, agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "term", "one");
    await agent.started(1);
    sessions.input(id, "term", "look [image 1]", ["/a.png"]);
    sessions.input(id, "term", "and [image 1] too", ["/b.png"]);
    sessions.input(id, "term", "/claude review");
    sessions.input(id, "term", "after");
    agent.release();
    await agent.started(2);
    expect(agent.prompts[1]).toMatchObject({
      app: "term",
      text: "look [image 1]\n\nand [image 2] too",
      images: ["/a.png", "/b.png"],
    });
    agent.release();
    await agent.started(3);
    expect(agent.prompts[2]?.text).toBe("/claude review");
    agent.release();
    await agent.started(4);
    expect(agent.prompts[3]?.text).toBe("after");
    agent.release();
    await sessions.idle(id);
    const users = store
      .read(id)
      .map((r) => r.event)
      .filter((e) => e.type === "user_message");
    expect(users).toHaveLength(4);
  });

  test("a running turn takes the inputs behind it up to a command, and logs them as it does", async () => {
    const { store, agent, sessions } = setup(undefined, true);
    const { id } = sessions.create({});
    sessions.input(id, "cli", "one");
    await agent.started(1);
    sessions.input(id, "term", "also this", ["/tmp/a.png"]);
    sessions.input(id, "term", "and this");
    sessions.input(id, "term", "/claude review it");
    sessions.input(id, "term", "after the command");
    expect(sessions.info(id)).toMatchObject({
      queued: 4,
      waiting: [
        { app: "term", text: "also this", images: ["/tmp/a.png"] },
        { app: "term", text: "and this" },
        { app: "term", text: "/claude review it" },
        { app: "term", text: "after the command" },
      ],
    });

    const take = agent.prompts[0]?.takeInputs;
    expect(take?.()).toEqual([{ text: "also this", images: ["/tmp/a.png"] }, { text: "and this" }]);
    expect(take?.()).toEqual([]);
    expect(sessions.info(id)?.queued).toBe(2);
    const users = store
      .read(id)
      .map((r) => r.event)
      .filter((e): e is UserMessage => e.type === "user_message")
      .map((e) => e.text);
    expect(users).toEqual(["one", "also this", "and this"]);

    // The command gets a turn of its own, and what came after it the next.
    agent.release();
    await agent.started(2);
    expect(agent.prompts[1]?.text).toBe("/claude review it");
    agent.release();
    await agent.started(3);
    expect(agent.prompts[2]?.text).toBe("after the command");
    agent.release();
    await sessions.idle(id);
    expect(sessions.info(id)?.waiting).toBeUndefined();
  });

  test("runs one turn at a time, in input order", async () => {
    const { store, agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "one");
    sessions.input(id, "overlay", "two");
    sessions.input(id, "cli", "three");
    expect(sessions.info(id)).toMatchObject({ state: "running", queued: 2 });

    await agent.started(1);
    expect(agent.prompts.map((p) => p.text)).toEqual(["one"]);
    agent.release();
    await agent.started(2);
    expect(sessions.info(id)?.queued).toBe(1);
    agent.release();
    await agent.started(3);
    agent.release();
    await sessions.idle(id);

    expect(agent.maxActive).toBe(1);
    expect(agent.prompts.map((p) => [p.app, p.text])).toEqual([
      ["cli", "one"],
      ["overlay", "two"],
      ["cli", "three"],
    ]);
    expect(types(store, id)).toEqual([
      ...["user_message", "agent_message_chunk", "turn_ended"],
      ...["user_message", "agent_message_chunk", "turn_ended"],
      ...["user_message", "agent_message_chunk", "turn_ended"],
    ]);
    expect(sessions.info(id)).toMatchObject({ state: "idle", queued: 0 });
  });

  test("input arriving during a turn waits for it", async () => {
    const { agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "first");
    await agent.started(1);
    sessions.input(id, "cli", "second");
    await Bun.sleep(5);
    expect(agent.prompts).toHaveLength(1);
    agent.release();
    await agent.started(2);
    agent.release();
    await sessions.idle(id);
    expect(agent.maxActive).toBe(1);
  });

  test("sessions run independently", async () => {
    const { agent, sessions } = setup();
    const a = sessions.create({}).id;
    const b = sessions.create({}).id;
    sessions.input(a, "cli", "a");
    sessions.input(b, "cli", "b");
    await agent.started(2);
    expect(agent.maxActive).toBe(2);
    agent.release();
    agent.release();
    await Promise.all([sessions.idle(a), sessions.idle(b)]);
  });

  test("turn_ended carries the stop reason and error", async () => {
    const { store, agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "x");
    await agent.started(1);
    agent.release({ stopReason: "error", error: "rate limited" });
    await sessions.idle(id);
    expect(store.read(id).at(-1)?.event).toEqual({
      type: "turn_ended",
      stopReason: "error",
      error: "rate limited",
    });
  });

  test("cancel ends the running turn; queued input still runs", async () => {
    const { store, agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "long");
    sessions.input(id, "cli", "next");
    await agent.started(1);
    sessions.cancel(id);
    await agent.started(2);
    agent.release();
    await sessions.idle(id);
    const ended = store
      .read(id)
      .map((r) => r.event)
      .filter((e): e is Extract<SessionEvent, { type: "turn_ended" }> => e.type === "turn_ended");
    expect(ended.map((e) => e.stopReason)).toEqual(["cancelled", "end_turn"]);
  });

  test("cancel when idle is a no-op; unknown sessions throw NotFoundError", () => {
    const { sessions } = setup();
    const { id } = sessions.create({});
    sessions.cancel(id);
    expect(() => sessions.input("s_nope", "cli", "x")).toThrow(NotFoundError);
  });

  test("updates after the turn ended are dropped", async () => {
    const store = new SessionStore(":memory:");
    const leaked: ((u: {
      type: "agent_message_chunk";
      messageId: string;
      text: string;
    }) => void)[] = [];
    const sessions = new SessionManager(
      store,
      {
        name: "leaky",
        newSession: () =>
          Promise.resolve({
            prompt: ({ onUpdate }) => {
              leaked.push(onUpdate);
              return Promise.resolve({ stopReason: "end_turn" as const });
            },
            cancel: () => {},
          }),
        loadSession: () => Promise.reject(new Error("unused")),
      },
      { defaultCwd: "/", log: silentLogger },
    );
    const { id } = sessions.create({});
    sessions.input(id, "cli", "x");
    await sessions.idle(id);
    leaked[0]?.({ type: "agent_message_chunk", messageId: "m", text: "late" });
    expect(types(store, id)).toEqual(["user_message", "turn_ended"]);
  });

  test("a failing agent load still logs the input and ends the turn with an error", async () => {
    const store = new SessionStore(":memory:");
    const sessions = new SessionManager(
      store,
      {
        name: "broken",
        newSession: () => Promise.reject(new Error("no model configured")),
        loadSession: () => Promise.reject(new Error("no model configured")),
      },
      { defaultCwd: "/", log: silentLogger },
    );
    const { id } = sessions.create({});
    sessions.input(id, "cli", "hi");
    await sessions.idle(id);
    expect(store.read(id).map((r) => r.event)).toEqual([
      { type: "user_message", app: "cli", text: "hi" },
      { type: "turn_ended", stopReason: "error", error: "no model configured" },
    ]);
  });
});

describe("agent sessions", () => {
  test("new sessions use newSession with the session cwd", async () => {
    const { agent, sessions } = setup();
    const { id } = sessions.create({ cwd: "/work" });
    sessions.input(id, "cli", "x");
    await agent.started(1);
    agent.release();
    await sessions.idle(id);
    expect(agent.created).toEqual([{ sessionId: id, cwd: "/work", mcpServers: [] }]);
    expect(sessions.info(id)?.cwd).toBe("/work");
  });

  test("after a restart the agent session is loaded from the full history", async () => {
    const store = new SessionStore(":memory:");
    const first = setup(store);
    const { id } = first.sessions.create({});
    first.sessions.input(id, "cli", "before");
    await first.agent.started(1);
    first.agent.release();
    await first.sessions.idle(id);

    const second = setup(store);
    second.sessions.input(id, "cli", "after");
    await second.agent.started(1);
    second.agent.release();
    await second.sessions.idle(id);
    expect(second.agent.loaded).toHaveLength(1);
    expect(second.agent.loaded[0]?.history.map((e) => e.type)).toEqual([
      "user_message",
      "agent_message_chunk",
      "turn_ended",
    ]);
  });

  test("recoverInterruptedTurns closes a turn cut off by a crash", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "s", cwd: "/" });
    store.append("s", { type: "user_message", app: "cli", text: "x" });
    store.append("s", { type: "agent_message_chunk", messageId: "m", text: "partial" });
    const { sessions } = setup(store);
    sessions.recoverInterruptedTurns();
    sessions.recoverInterruptedTurns(); // idempotent
    expect(store.read("s").map((r) => r.event.type)).toEqual([
      "user_message",
      "agent_message_chunk",
      "turn_ended",
    ]);
  });

  test("recoverInterruptedTurns drops questions nobody can answer any more", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "s", cwd: "/" });
    const options = [{ id: "a", name: "A" }];
    store.append("s", { type: "choice", id: "c_open", title: "Settings", options });
    store.append("s", { type: "choice", id: "c_done", title: "Model", options });
    store.append("s", { type: "choice_made", id: "c_done", option: "a" });
    store.append("s", {
      type: "permission_request",
      id: "p_open",
      agent: "claude",
      title: "Run ls",
      options: [{ id: "allow", name: "Allow", kind: "allow_once" }],
    });
    const { sessions } = setup(store);
    sessions.recoverInterruptedTurns();
    sessions.recoverInterruptedTurns(); // idempotent
    expect(store.read("s", 5).map((r) => r.event)).toEqual([
      { type: "choice_made", id: "c_open" },
      { type: "permission_resolved", id: "p_open" },
    ]);
  });
});

describe("pending context events", () => {
  test("connector events since the last prompt go to the next prompt, once", async () => {
    const { store, agent, sessions } = setup();
    const { id } = sessions.create({});
    sessions.input(id, "cli", "first");
    await agent.started(1);
    store.append(id, { type: "app_connected", app: "lsp", kind: "service", v: 1, tools: ["x"] });
    agent.release();
    store.append(id, { type: "focus_changed", app: "browser", context: { url: "u" } });
    sessions.input(id, "cli", "second");
    await agent.started(2);
    agent.release();
    sessions.input(id, "cli", "third");
    await agent.started(3);
    agent.release();
    await sessions.idle(id);

    expect(agent.prompts.map((p) => p.pendingEvents.map((e) => e.type))).toEqual([
      [],
      ["app_connected", "focus_changed"],
      [],
    ]);
  });

  test("pending events survive a restart", async () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "s", cwd: "/" });
    store.append("s", { type: "user_message", app: "cli", text: "old" });
    store.append("s", { type: "turn_ended", stopReason: "end_turn" });
    store.append("s", { type: "app_disconnected", app: "photo-editor" });
    const { agent, sessions } = setup(store);
    sessions.input("s", "cli", "new");
    await agent.started(1);
    agent.release();
    await sessions.idle("s");
    expect(agent.prompts[0]?.pendingEvents).toEqual([
      { type: "app_disconnected", app: "photo-editor" },
    ]);
  });
});

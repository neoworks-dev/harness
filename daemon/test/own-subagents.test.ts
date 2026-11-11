// Subagents: the agents' own (Claude Code's Agent tool, Codex's) beside neo's delegate, the
// user's settings for them, and the approvals mode they work in.
import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { withCodexSubagents } from "../src/guests/config.ts";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
  subagents: "asks",
};

async function setup(guest: Partial<GuestConfig> = {}): Promise<TestDaemon> {
  const daemon = await startTestDaemon({
    agent: new FakeAgent("main"),
    guests: { fake: { ...FAKE, ...guest } },
  });
  cleanups.push(() => daemon.stop());
  return daemon;
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** The session's agent's last reply. */
function reply(daemon: TestDaemon, session: string): string {
  const texts = new Map<string, string>();
  for (const e of events(daemon, session)) {
    if (e.type === "agent_message_chunk" && e.agent === undefined) {
      texts.set(e.messageId, (texts.get(e.messageId) ?? "") + e.text);
    }
  }
  return [...texts.values()].at(-1) ?? "";
}

/** A session that runs on the fake guest, after `before` set things up. */
async function onFake(daemon: TestDaemon): Promise<string> {
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/harness fake");
  return session;
}

async function say(daemon: TestDaemon, session: string, text: string): Promise<void> {
  daemon.sessions.input(session, "cli", text);
  await daemon.sessions.idle(session);
}

describe("the agents' own subagents", () => {
  test("Claude Code's Agent tool is on for the session's agent and asks; off for delegate only", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    await say(daemon, session, "config");
    const on = JSON.parse(reply(daemon, session));
    expect(on.ask).toEqual(["Agent", "Task"]);
    expect(on.disallowed).toBeUndefined();

    daemon.settings.setSubagents("delegate");
    const other = await onFake(daemon);
    await say(daemon, other, "config");
    const off = JSON.parse(reply(daemon, other));
    expect(off.disallowed).toEqual(["Agent", "Task"]);
    expect(off.ask).toBeUndefined();
  });

  test("Codex's start without asking: on unless every start is to be asked about", async () => {
    const daemon = await setup({ subagents: "silent", env: { CODEX_CONFIG: "{}" } });
    const features = async (): Promise<unknown> => {
      const session = await onFake(daemon);
      await say(daemon, session, "route");
      return JSON.parse(JSON.parse(reply(daemon, session)).env.CODEX_CONFIG).features;
    };
    expect(await features()).toEqual({ multi_agent: true });
    daemon.settings.setAlwaysAskForAgents(true);
    expect(await features()).toEqual({ multi_agent: false });
  });

  test("a start asks like delegate's; auto mode allows it unless the user wants it asked", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    daemon.sessions.input(session, "cli", "ask agent");
    await until(
      () => events(daemon, session).some((e) => e.type === "permission_request"),
      "the question",
    );
    const asked = events(daemon, session).find((e) => e.type === "permission_request");
    expect(asked).toMatchObject({
      title: "Start a subagent?",
      details: [
        { key: "Agent", value: "Explore" },
        { key: "Task", value: "Find the tests" },
      ],
      content: [{ type: "text", text: "List the test files." }],
    });
    if (asked?.type !== "permission_request") throw new Error("no question");
    expect(asked.options.map((o) => o.id)).toEqual(["yes", "always", "no"]);
    await daemon.actions.run(
      { session, ref: "x", action: "neo.permit", args: { id: asked.id, option: "yes" } },
      "test",
    );
    await daemon.sessions.idle(session);
    expect(reply(daemon, session)).toBe("answer: yes");

    await daemon.commands.run(session, "/approvals auto");
    await say(daemon, session, "ask agent");
    expect(reply(daemon, session)).toBe("answer: yes");
    expect(events(daemon, session).filter((e) => e.type === "permission_request")).toHaveLength(1);

    // Asked in auto mode too, and nothing to allow it for good with.
    daemon.settings.setAlwaysAskForAgents(true);
    daemon.sessions.input(session, "cli", "ask agent");
    await until(
      () => events(daemon, session).filter((e) => e.type === "permission_request").length === 2,
      "the second question",
    );
    const again = events(daemon, session).findLast((e) => e.type === "permission_request");
    if (again?.type !== "permission_request") throw new Error("no question");
    expect(again.options.map((o) => o.id)).toEqual(["yes", "no"]);
    await daemon.actions.run(
      { session, ref: "x", action: "neo.permit", args: { id: again.id, option: "no" } },
      "test",
    );
    await daemon.sessions.idle(session);
  });
});

describe("the agents' own subagents show like delegate's", () => {
  test("the adapter is told it may report them as sessions of their own", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    await say(daemon, session, "route");
    const route = JSON.parse(reply(daemon, session));
    expect(route.initialize.clientCapabilities.subagents).toEqual({});
  });

  test("Claude Code's (the ACP RFD's form): a start, its own labelled work and transcript, an end", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    await say(daemon, session, "subagent");
    const all = events(daemon, session);
    const started = all.find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started") throw new Error("no subagent_started");
    expect(started).toMatchObject({ name: "Explore", task: "List them.", native: true });
    expect(started.agent).toMatch(/^fake\/[0-9a-f]{8}$/);
    expect(started.background).toBeUndefined();
    expect(started.handle).toBe("sub-1");
    // Only a delegation into a directory gets a session of its own.
    expect(started.child).toBeUndefined();
    expect(daemon.sessions.list().filter((s) => s.parent === session)).toEqual([]);
    const ended = all.find((e) => e.type === "subagent_ended");
    expect(ended).toEqual({ type: "subagent_ended", id: started.id, stopReason: "end_turn" });
    // The subagent's work carries its label; the session's agent's reply doesn't.
    const glob = all.find((e) => e.type === "tool_call" && e.title === "Glob test/**");
    expect(glob).toMatchObject({ agent: started.agent });
    expect(reply(daemon, session)).toBe("It found them.");

    // Its transcript: the task, then its work; the session's view: the Agent call, like a
    // delegation, without the subagent's work.
    const own = daemon.views.get(session, started.agent).snapshot().items;
    expect(JSON.stringify(own)).toContain("List them.");
    expect(JSON.stringify(own)).toContain("Found 3 tests.");
    expect(JSON.stringify(own)).not.toContain("It found them.");
    const main = daemon.views.get(session).snapshot().items;
    expect(main.map((i) => i.label)).toContain("Explore · Find the tests");
    expect(JSON.stringify(main)).not.toContain("Found 3 tests.");
  });

  test("a subagent's question is its own, and the answer reaches its session", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    daemon.sessions.input(session, "cli", "subagent ask");
    await until(
      () => events(daemon, session).some((e) => e.type === "permission_request"),
      "the question",
    );
    const started = events(daemon, session).find((e) => e.type === "subagent_started");
    const asked = events(daemon, session).find((e) => e.type === "permission_request");
    if (started?.type !== "subagent_started" || asked?.type !== "permission_request") {
      throw new Error("no subagent question");
    }
    expect(asked.agent).toBe(started.agent);
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${asked.id}`);
    expect(item?.label).toBe("Explore: rm x");
    await daemon.actions.run(
      { session, ref: "x", action: "neo.permit", args: { id: asked.id, option: "no" } },
      "test",
    );
    await daemon.sessions.idle(session);
    const own = daemon.views.get(session, started.agent).snapshot().items;
    expect(JSON.stringify(own)).toContain("asked: no");
  });

  test("a turn left open for a subagent in the background says it only waits on it", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    daemon.sessions.input(session, "cli", "background subagent");
    // Its reply is done and the subagent works on: the turn runs, waiting on it.
    await until(
      () => daemon.sessions.info(session)?.waitingOn !== undefined,
      "the turn to wait on the subagent",
    );
    expect(daemon.sessions.info(session)).toMatchObject({
      state: "running",
      waitingOn: ["Research the cache"],
    });
    // The subagent done and the agent writing again: no longer only waiting.
    await daemon.sessions.idle(session);
    expect(daemon.sessions.info(session)?.waitingOn).toBeUndefined();
    expect(reply(daemon, session)).toContain("It says five minutes.");
  });

  test("an Agent call without a subagent_type is known to run in the background too", async () => {
    const daemon = await setup();
    const session = await onFake(daemon);
    daemon.sessions.input(session, "cli", "background subagent untyped");
    await daemon.sessions.idle(session);
    const started = events(daemon, session).find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started") throw new Error("no subagent_started");
    // Not background, the turn's end would forget it and its work would show in the main chat.
    expect(started.background).toBe(true);
  });

  test("Codex's (AIR's form): spawned with its prompt, and its failure ends it", async () => {
    const daemon = await setup({ subagents: "silent" });
    const session = await onFake(daemon);
    await say(daemon, session, "spawn");
    const all = events(daemon, session);
    const started = all.find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started") throw new Error("no subagent_started");
    expect(started).toMatchObject({ name: "worker", task: "Count the lines.", native: true });
    expect(
      all.find((e) => e.type === "agent_message_chunk" && e.text === "42 lines."),
    ).toMatchObject({ agent: started.agent });
    expect(all.find((e) => e.type === "subagent_ended")).toMatchObject({
      id: started.id,
      stopReason: "error",
    });
  });
});

describe("delegate and the settings", () => {
  test("always ask: auto mode asks, without an allow for the session", async () => {
    const daemon = await setup();
    const session = daemon.sessions.create({}).id;
    await daemon.commands.run(session, "/approvals auto");
    daemon.settings.setAlwaysAskForAgents(true);
    const call = daemon.gateway.call("delegate", { agent: "fake", task: "x" }, undefined, session);
    await until(
      () => events(daemon, session).some((e) => e.type === "permission_request"),
      "the question",
    );
    const asked = events(daemon, session).find((e) => e.type === "permission_request");
    if (asked?.type !== "permission_request") throw new Error("no question");
    expect(asked.options.map((o) => o.id)).not.toContain("always");
    await daemon.actions.run(
      { session, ref: "x", action: "neo.permit", args: { id: asked.id, option: "reject" } },
      "test",
    );
    expect((await call).isError).toBe(true);
  });

  test("own subagents only: an agent that has them is refused delegate; neo keeps it", async () => {
    const daemon = await setup();
    daemon.settings.setSubagents("native");
    const session = await onFake(daemon);
    const refused = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "x" },
      undefined,
      session,
    );
    expect(refused.isError).toBe(true);
    expect(JSON.stringify(refused.content)).toContain("use your own subagent tool");

    // A new session starts on the last harness picked: back to the daemon's own agent.
    const neo = daemon.sessions.create({}).id;
    await daemon.commands.run(neo, "/harness main");
    await daemon.commands.run(neo, "/approvals auto");
    const ran = await daemon.gateway.call("delegate", { agent: "fake", task: "x" }, undefined, neo);
    expect(JSON.stringify(ran.content)).not.toContain("use your own subagent tool");
    expect(ran.isError, JSON.stringify(ran.content)).toBeUndefined();
  });
});

test("a subagent's session follows its parent's approvals mode", async () => {
  const daemon = await setup();
  const parent = daemon.sessions.create({}).id;
  const child = daemon.sessions.create({ parent, cwd: "/tmp" }).id;
  const grandchild = daemon.sessions.create({ parent: child, cwd: "/tmp" }).id;
  daemon.settings.setApprovals(parent, "edits");
  expect(daemon.settings.approvals(child)).toBe("edits");
  expect(daemon.settings.approvals(grandchild)).toBe("edits");
  // A child's own choice holds until the parent's changes again.
  daemon.settings.setApprovals(child, "ask");
  expect(daemon.settings.approvals(parent)).toBe("edits");
  daemon.settings.setApprovals(parent, "auto");
  expect(daemon.settings.approvals(child)).toBe("auto");
});

test("CODEX_CONFIG keeps the user's config and sets only multi_agent", () => {
  const json = JSON.stringify({ model: "o3", features: { web_search: true, multi_agent: false } });
  expect(JSON.parse(withCodexSubagents(json, true))).toEqual({
    model: "o3",
    features: { web_search: true, multi_agent: true },
  });
  expect(JSON.parse(withCodexSubagents("not json", false))).toEqual({
    features: { multi_agent: false },
  });
});

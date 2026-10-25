// Messaging between agents through real subagents: background delegation, steering a running
// subagent, a subagent writing to the session's agent, how each kind of agent receives (an ACP
// agent with its tool results, neo as user messages), and what happens when a turn is cancelled.
import { afterEach, describe, expect, test } from "bun:test";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { createNeoAgent } from "@neoworks-dev/neo/neo";
import { fauxToolCall } from "@mariozechner/pi-ai";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { fakeModel, reply, TEST_CONFIG } from "../../harness/test/fake-model.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

/** The user allows every delegation. */
function allowDelegations(daemon: TestDaemon): void {
  daemon.store.onAppend(({ session, event }) => {
    if (event.type !== "permission_request" || event.title !== "Start a subagent?") return;
    void Promise.resolve().then(() =>
      daemon.actions.run(
        { session, ref: "p", action: "suite.permit", args: { id: event.id, option: "allow" } },
        "test",
      ),
    );
  });
}

async function setup(): Promise<{ daemon: TestDaemon; session: string; main: FakeAgent }> {
  const main = new FakeAgent("main");
  const daemon = await startTestDaemon({ agent: main, guests: { fake: FAKE } });
  cleanups.push(() => daemon.stop());
  allowDelegations(daemon);
  return { daemon, session: daemon.sessions.create({}).id, main };
}

const events = (daemon: TestDaemon, session: string): SessionEvent[] =>
  daemon.store.read(session).map((r) => r.event);

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

const resultOf = (result: CallToolResult): string => (result.content[0] as { text: string }).text;

function delegate(
  daemon: TestDaemon,
  session: string,
  args: Record<string, unknown>,
): Promise<CallToolResult> {
  return daemon.gateway.call("delegate", { agent: "fake", task: "x", ...args }, undefined, session);
}

/** The subagent's id, from the log. */
function startedId(daemon: TestDaemon, session: string): string {
  const started = events(daemon, session).find((e) => e.type === "subagent_started");
  if (started?.type !== "subagent_started") throw new Error("no subagent started");
  return started.id;
}

/** Until the subagent has begun its turn (it echoes its prompt as a thought). */
async function working(daemon: TestDaemon, session: string): Promise<string> {
  await until(
    () => events(daemon, session).some((e) => e.type === "agent_thought_chunk"),
    "the subagent to start",
  );
  return startedId(daemon, session);
}

type AgentMessage = Extract<SessionEvent, { type: "agent_message" }>;
const messages = (daemon: TestDaemon, session: string): AgentMessage[] =>
  events(daemon, session).filter((e): e is AgentMessage => e.type === "agent_message");

describe("a background delegation", () => {
  test("returns at once with the id; a message steers it; its reply wakes the idle agent", async () => {
    const { daemon, session, main } = await setup();
    const started = await delegate(daemon, session, { task: "listen", background: true });
    const id = startedId(daemon, session);
    expect(resultOf(started)).toContain(`Started fake as ${id} in the background`);
    expect(events(daemon, session).find((e) => e.type === "subagent_started")).toMatchObject({
      id,
      background: true,
    });
    expect(events(daemon, session).some((e) => e.type === "subagent_ended")).toBe(false);

    // The subagent is in its wait: a message from the session's agent ends it.
    await working(daemon, session);
    const sent = await daemon.gateway.call(
      "send_message",
      { to: "fake", text: "use the staging db" },
      undefined,
      session,
    );
    expect(resultOf(sent)).toBe(`Sent to ${id} (fake).`);

    // Its final reply is a message to the agent, which is idle: that starts a turn.
    await main.started(1);
    const prompt = main.prompts[0]?.text ?? "";
    expect(prompt).toContain(`<message from="${id}" agent="fake" reply="true">`);
    expect(prompt).toContain('heard: <message from="main">');
    expect(prompt).toContain("use the staging db");
    expect(main.prompts[0]?.app).toBe("agents");
    expect(messages(daemon, session)).toMatchObject([
      { from: "main", to: id, toAgent: "fake", text: "use the staging db" },
      { from: id, to: "main", fromAgent: "fake", reply: true },
    ]);
    expect(events(daemon, session).find((e) => e.type === "subagent_ended")).toMatchObject({
      id,
      stopReason: "end_turn",
    });
    main.release();
    await daemon.sessions.idle(session);
  });

  test("an ACP agent gets a message with the result of its next suite tool", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "poll", background: true });
    const id = await working(daemon, session);
    await daemon.gateway.call("send_message", { to: id, text: "stop polling" }, undefined, session);
    await main.started(1);
    expect(main.prompts[0]?.text).toContain("polled: ");
    expect(main.prompts[0]?.text).toContain("stop polling");
    main.release();
    await daemon.sessions.idle(session);
  });

  test("a message that came before its turn started comes with its first prompt", async () => {
    const { daemon, session } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    const id = startedId(daemon, session);
    // The adapter takes a moment to start: the message is there before the turn is.
    await daemon.gateway.call("send_message", { to: id, text: "early word" }, undefined, session);
    await working(daemon, session);
    const thought = events(daemon, session).find((e) => e.type === "agent_thought_chunk");
    let echoed = "";
    if (thought?.type === "agent_thought_chunk") echoed = thought.text;
    expect(echoed).toContain('<message from="main">\nearly word\n</message>');
    daemon.sessions.cancel(session);
    await until(() => events(daemon, session).some((e) => e.type === "subagent_ended"), "end");
  });

  test("one subagent per harness: a second delegation says to message the first", async () => {
    const { daemon, session } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    const id = startedId(daemon, session);
    const second = await delegate(daemon, session, { task: "other", background: true });
    expect(second.isError).toBe(true);
    expect(resultOf(second)).toContain(`still working on an earlier task here (${id})`);
    await daemon.actions.run(
      { session, ref: "a", action: "suite.cancel_agent", args: { id } },
      "test",
    );
    await until(() => events(daemon, session).some((e) => e.type === "subagent_ended"), "end");
    // Free again.
    const third = await delegate(daemon, session, { task: "again", background: true });
    expect(third.isError).toBeUndefined();
  });

  test("a subagent that has ended can't be written to, and says so", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "quick", background: true });
    const id = startedId(daemon, session);
    await main.started(1);
    const late = await daemon.gateway.call(
      "send_message",
      { to: id, text: "too late" },
      undefined,
      session,
    );
    expect(late.isError).toBe(true);
    expect(resultOf(late)).toBe(`fake ${id} has ended (finished); it can't read messages.`);
    main.release();
    await daemon.sessions.idle(session);
  });
});

describe("a delegation the agent waits for", () => {
  test("can't write to the agent, which can't answer; it says so", async () => {
    const { daemon, session } = await setup();
    const result = await delegate(daemon, session, { task: "send main are you there" });
    expect(resultOf(result)).toContain("refused: main is waiting for you to finish");
    expect(messages(daemon, session)).toEqual([]);
  });

  test("a message to it is taken with its next tool result", async () => {
    const { daemon, session } = await setup();
    const call = delegate(daemon, session, { task: "poll" });
    const id = await working(daemon, session);
    await daemon.gateway.call(
      "send_message",
      { to: id, text: "from the side" },
      undefined,
      session,
    );
    expect(resultOf(await call)).toContain("from the side");
  });
});

describe("the tools", () => {
  test("check their arguments and who calls", async () => {
    const { daemon, session } = await setup();
    const call = (
      name: "send_message" | "wait" | "delegate",
      args: Record<string, unknown>,
    ): Promise<CallToolResult> => daemon.gateway.call(name, args, undefined, session);
    expect(resultOf(await call("send_message", { text: "x" }))).toBe(
      "send_message: to is required",
    );
    expect(resultOf(await call("send_message", { to: "main", text: " " }))).toBe(
      "send_message: text is required",
    );
    expect(resultOf(await call("wait", { for: "d_1" }))).toBe(
      "wait: for must be a list of subagent ids",
    );
    expect(resultOf(await call("wait", { timeoutSeconds: "soon" }))).toBe(
      "wait: timeoutSeconds must be a number",
    );
    expect(resultOf(await call("delegate", { agent: "fake", task: "x", background: "yes" }))).toBe(
      "delegate: background must be a boolean",
    );
    // The agent writing to itself, and to nobody.
    expect(resultOf(await call("send_message", { to: "main", text: "me" }))).toContain(
      "That's you",
    );
    expect(resultOf(await call("send_message", { to: "fake", text: "you" }))).toContain(
      'No running subagent "fake". No subagent is running.',
    );
    // A guest that no delegation started (the user's own /claude turn) is nobody.
    const stranger = await daemon.gateway.call(
      "send_message",
      { to: "main", text: "hi" },
      undefined,
      { session, guest: "fake", text: true },
    );
    expect(stranger.isError).toBe(true);
    expect(resultOf(stranger)).toContain("for the session's agent and the subagents");
  });

  test("a subagent's delegate is refused", async () => {
    const { daemon, session } = await setup();
    const asGuest = { session, guest: "fake", text: true };
    const refused = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "x" },
      undefined,
      asGuest,
    );
    expect(resultOf(refused)).toBe("Subagents can't delegate.");
  });
});

describe("limits", () => {
  test("a subagent that sends too many gets the limit as the answer", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "spam main", background: true });
    await until(() => messages(daemon, session).some((m) => m.reply === true), "the reply");
    const reply = messages(daemon, session).find((m) => m.reply === true);
    expect(reply?.text).toContain("taken: 20; last: Message limit reached: 20 messages from");
    // The first message started a turn; what came meanwhile starts the next, all in one.
    main.release();
    await main.started(2);
    const second = main.prompts[1]?.text ?? "";
    expect(second.match(/<message /g)).toHaveLength(20);
    expect(second).toContain("ping 19");
    main.release();
    await daemon.sessions.idle(session);
  });
});

describe("stopping", () => {
  test("cancelling the turn stops background subagents; the agent hears so without a turn", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    const id = await working(daemon, session);
    daemon.sessions.cancel(session);
    await until(() => events(daemon, session).some((e) => e.type === "subagent_ended"), "end");
    expect(events(daemon, session).find((e) => e.type === "subagent_ended")).toMatchObject({
      id,
      stopReason: "cancelled",
    });
    await Bun.sleep(50);
    expect(main.prompts).toHaveLength(0);
    // It reads it whenever it next looks.
    const waited = await daemon.gateway.call("wait", {}, undefined, session);
    expect(resultOf(waited)).toContain("[fake stopped: cancelled]");
  });

  test("a turn that simply ends leaves them running", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    const id = await working(daemon, session);
    daemon.sessions.input(session, "cli", "hello");
    await main.started(1);
    main.release();
    await daemon.sessions.idle(session);
    expect(events(daemon, session).some((e) => e.type === "subagent_ended")).toBe(false);
    await daemon.actions.run(
      { session, ref: "a", action: "suite.cancel_agent", args: { id } },
      "test",
    );
    await until(() => events(daemon, session).some((e) => e.type === "subagent_ended"), "end");
  });

  test("x on one stops it and the agent is told, quietly", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    const id = await working(daemon, session);
    await daemon.actions.run(
      { session, ref: "a", action: "suite.cancel_agent", args: { id } },
      "test",
    );
    await until(() => messages(daemon, session).some((m) => m.reply === true), "the report");
    await Bun.sleep(50);
    expect(main.prompts).toHaveLength(0);
  });

  test("one that was running when the daemon died is closed when it starts again", async () => {
    const first = await startTestDaemon({});
    const session = first.sessions.create({}).id;
    first.store.append(session, { type: "user_message", app: "cli", text: "go" });
    first.store.append(session, {
      type: "subagent_started",
      id: "d_gone",
      agent: "fake",
      task: "x",
      background: true,
    });
    await first.stop();
    const again = await startTestDaemon({ dir: first.dir });
    cleanups.push(() => again.stop());
    expect(events(again, session).at(-1)).toMatchObject({
      type: "subagent_ended",
      id: "d_gone",
      stopReason: "error",
    });
  });

  test("stopping the daemon stops them", async () => {
    const { daemon, session } = await setup();
    await delegate(daemon, session, { task: "wait", background: true });
    await working(daemon, session);
    const seen: SessionEvent[] = [];
    daemon.store.onAppend(({ event }) => seen.push(event));
    await daemon.stop();
    expect(seen.find((e) => e.type === "subagent_ended")).toMatchObject({
      stopReason: "cancelled",
    });
  });
});

describe("the chat", () => {
  test("shows the messages as rows, not the turn they start, and keeps a background subagent hidden", async () => {
    const { daemon, session, main } = await setup();
    await delegate(daemon, session, { task: "listen", background: true });
    const id = await working(daemon, session);
    // The main agent's turn ends while the subagent runs on: its work stays out of the chat.
    daemon.sessions.input(session, "cli", "hello");
    await main.started(1);
    main.release();
    await daemon.sessions.idle(session);
    await daemon.gateway.call("send_message", { to: id, text: "carry on" }, undefined, session);
    await main.started(2);
    const view = daemon.views.get(session).snapshot();
    const labels = view.items.map((item) => item.label ?? item.kind);
    expect(labels).toContain(`message: main → fake ${id}`);
    expect(labels.some((l) => l.startsWith("reply: fake"))).toBe(true);
    expect(view.items.filter((item) => item.kind === "user")).toHaveLength(1);
    expect(view.items.some((item) => item.kind === "thought" || item.label === "fake")).toBe(false);
    main.release();
    await daemon.sessions.idle(session);
  });
});

describe("neo as the subagent", () => {
  test("waits for a message with the wait tool, and reads it", async () => {
    const model = fakeModel([
      // The subagent: wait, then answer with what it heard.
      reply([fauxToolCall("wait", { timeoutSeconds: 10 }, { id: "w1" })], "toolUse"),
      (context) => {
        const result = context.messages.findLast((m) => m.role === "toolResult");
        const heard = JSON.stringify(result?.content ?? "");
        return reply(
          heard.includes("switch to y") ? "neo heard: switch to y" : "neo heard nothing",
        );
      },
      // The session's agent, woken by the reply.
      reply("noted"),
    ]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const daemon = await startTestDaemon({ agent: neo });
    cleanups.push(() => daemon.stop());
    allowDelegations(daemon);
    const session = daemon.sessions.create({}).id;
    const started = await daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "wait for instructions", background: true },
      undefined,
      session,
    );
    const id = startedId(daemon, session);
    expect(resultOf(started)).toContain(`Started neo as ${id}`);
    await until(() => model.requests.length >= 1, "the subagent's first request");
    await Bun.sleep(100);
    await daemon.gateway.call(
      "send_message",
      { to: "neo", text: "switch to y" },
      undefined,
      session,
    );
    await until(() => model.requests.length >= 3, "the woken agent");
    await daemon.sessions.idle(session);
    const reply2 = messages(daemon, session).find((m) => m.reply === true);
    expect(reply2?.text).toContain("neo heard: switch to y");
    // The woken agent was told in a turn of its own.
    const wake = events(daemon, session).find((e) => e.type === "user_message");
    expect(wake).toMatchObject({ app: "agents" });
  });
});

describe("a subagent in a directory of its own", () => {
  test("hears messages as the agent of its session; its reply comes back to the agent", async () => {
    const { daemon, session, main } = await setup();
    mkdirSync(join(daemon.dir, "sub"));
    const started = await delegate(daemon, session, {
      task: "listen",
      background: true,
      cwd: "sub",
    });
    const id = startedId(daemon, session);
    const begun = events(daemon, session).find((e) => e.type === "subagent_started");
    const child = begun?.type === "subagent_started" ? begun.child : undefined;
    expect(child).toBeDefined();
    expect(resultOf(started)).toContain(`session ${child}`);
    await until(
      () => events(daemon, child ?? "").some((e) => e.type === "agent_thought_chunk"),
      "the child's turn",
    );
    await daemon.gateway.call("send_message", { to: id, text: "go on" }, undefined, session);
    await main.started(1);
    expect(main.prompts[0]?.text).toContain("heard: ");
    expect(main.prompts[0]?.text).toContain("go on");
    main.release();
    await daemon.sessions.idle(session);
  });

  test("a message that arrives as its turn ends starts one more, and the reply is the last turn's", async () => {
    const { daemon, session, main } = await setup();
    mkdirSync(join(daemon.dir, "sub"));
    await delegate(daemon, session, { task: "quick", background: true, cwd: "sub" });
    const id = startedId(daemon, session);
    const begun = events(daemon, session).find((e) => e.type === "subagent_started");
    let child = "";
    if (begun?.type === "subagent_started") child = begun.child ?? "";
    await until(() => daemon.sessions.info(child)?.state === "running", "the child's turn");
    // Whenever it lands, the subagent doesn't end with it unread.
    await daemon.gateway.call("send_message", { to: id, text: "listen" }, undefined, session);
    await main.started(1);
    const reply = messages(daemon, session).find((m) => m.reply === true);
    expect(reply).toBeDefined();
    const turns = events(daemon, child).filter((e) => e.type === "turn_ended");
    expect(turns.length).toBeGreaterThanOrEqual(1);
    main.release();
    await daemon.sessions.idle(session);
  });
});

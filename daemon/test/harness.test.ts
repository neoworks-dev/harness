// The session's harness (/harness): turns run on neo or on an ACP harness (the fake ACP agent),
// switching between them, delegate with model and effort, and neo subagents.
import { afterEach, describe, expect, test } from "bun:test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { fauxToolCall } from "@mariozechner/pi-ai";
import { createNeoAgent } from "@suite/harness/neo";
import type { SessionEvent } from "@suite/protocol/types";
import { fakeModel, reply, TEST_CONFIG, withUsage } from "../../harness/test/fake-model.ts";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";
import { type AppTool, startMcpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};
const GUESTS = { fake: FAKE, other: { ...FAKE, title: "Other" } };

const greet: AppTool = {
  name: "greet",
  description: "Say hello to someone",
  inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
  handler: (args) => ({
    content: [{ type: "text", text: `Hello, ${String(args.name)}!` }],
    structuredContent: { greeted: args.name },
  }),
};

interface Setup {
  daemon: TestDaemon;
  session: string;
  agent: FakeAgent;
}

async function setup(
  options: {
    agent?: FakeAgent;
    neo?: ReturnType<typeof createNeoAgent>;
    /** How delegate's "start a subagent?" question is answered; manual: by the test. */
    gate?: string;
  } = {},
): Promise<Setup> {
  const agent = options.agent ?? new FakeAgent("main");
  const daemon = await startTestDaemon({ agent, guests: GUESTS, neo: options.neo });
  cleanups.push(() => daemon.stop());
  const gate = options.gate ?? "allow";
  if (gate !== "manual") {
    daemon.store.onAppend(({ session, event }) => {
      if (event.type !== "permission_request" || event.title !== "Start a subagent?") return;
      void Promise.resolve().then(() =>
        daemon.actions.run(
          { session, ref: "p", action: "suite.permit", args: { id: event.id, option: gate } },
          "test",
        ),
      );
    });
  }
  const app = startMcpApp({ tools: [greet] });
  cleanups.push(() => app.close());
  await daemon.connectors.presence(
    {},
    {
      app: "greeter",
      kind: "app",
      open: true,
      mcp: app.endpoint,
      templates: { greet: { type: "text", markdown: "Greeted **{{data.greeted}}**" } },
    },
  );
  return { daemon, session: daemon.sessions.create({}).id, agent };
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

/** The user's turn; resolves once the session is idle again. */
async function say(daemon: TestDaemon, session: string, text: string): Promise<void> {
  daemon.sessions.input(session, "cli", text);
  await daemon.sessions.idle(session);
}

/** The text of the thoughts (the fake ACP agent echoes each prompt as one), per turn. */
function thoughts(daemon: TestDaemon, session: string): string[] {
  const texts: string[] = [];
  for (const e of events(daemon, session)) {
    if (e.type === "agent_thought_chunk") texts.push(e.text);
  }
  return texts;
}

/** The agent's message chunks: the session's own (no agent), or a subagent's. */
function replies(daemon: TestDaemon, session: string, agent?: string): string[] {
  const texts: string[] = [];
  for (const e of events(daemon, session)) {
    if (e.type === "agent_message_chunk" && e.agent === agent) texts.push(e.text);
  }
  return texts;
}

function resultOf(result: { content: unknown[] }): string {
  return (result.content[0] as { text: string }).text;
}

function answered(daemon: TestDaemon, session: string, event: SessionEvent): boolean {
  if (event.type !== "choice" && event.type !== "permission_request") return true;
  return events(daemon, session).some(
    (e) => (e.type === "choice_made" || e.type === "permission_resolved") && e.id === event.id,
  );
}

async function answer(
  daemon: TestDaemon,
  session: string,
  type: "permission_request" | "choice",
  option: string,
): Promise<SessionEvent> {
  await until(
    () => events(daemon, session).some((e) => e.type === type && !answered(daemon, session, e)),
    type,
  );
  const open = events(daemon, session).findLast((e) => e.type === type);
  if (open?.type !== type) throw new Error(`no ${type}`);
  const action = type === "choice" ? "suite.choose" : "suite.permit";
  await daemon.actions.run({ session, ref: "x", action, args: { id: open.id, option } }, "test");
  return open;
}

describe("/harness", () => {
  test("sets the harness: logged, shown in the chat, validated", async () => {
    const { daemon, session } = await setup();
    expect(daemon.settings.harness(session)).toBe("main");
    await daemon.commands.run(session, "/harness fake");
    expect(daemon.settings.harness(session)).toBe("fake");
    expect(events(daemon, session).filter((e) => e.type === "setting_changed")).toEqual([
      { type: "setting_changed", harness: "suite", option: "harness", value: "fake" },
    ]);
    // Unlike the approvals mode this one shows in the chat.
    const items = daemon.views.get(session).snapshot().items;
    expect(items.map((i) => JSON.stringify(i.component))).toEqual([
      expect.stringContaining("harness: fake"),
    ]);
    expect(daemon.commands.run(session, "/harness nope")).rejects.toThrow(
      "usage: /harness [main|fake|other]",
    );
    expect(daemon.commands.run(session, "/harness fake other")).rejects.toThrow("usage");
  });

  test("without a name it offers the harnesses; the pick applies", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness");
    const choice = await answer(daemon, session, "choice", "other");
    expect(choice).toMatchObject({
      title: "Harness (now: main)",
      options: [
        { id: "main", name: "main" },
        { id: "fake", name: "Fake" },
        { id: "other", name: "Other" },
      ],
    });
    await until(() => daemon.settings.harness(session) === "other", "the harness");
    await daemon.commands.run(session, "/harness");
    const again = await answer(daemon, session, "choice", "main");
    expect(again).toMatchObject({ title: "Harness (now: other)" });
  });

  test("the last choice is the default for new sessions; existing ones keep theirs", async () => {
    const { daemon, session } = await setup();
    const early = daemon.sessions.create({}).id;
    await daemon.commands.run(session, "/harness fake");
    expect(daemon.settings.harness(early)).toBe("main");
    const next = daemon.sessions.create({}).id;
    expect(daemon.settings.harness(next)).toBe("fake");
    // Logged, so panes see it and a later choice elsewhere doesn't move the session.
    expect(events(daemon, next)).toEqual([
      { type: "setting_changed", harness: "suite", option: "harness", value: "fake" },
    ]);
    await daemon.commands.run(early, "/harness other");
    expect(daemon.settings.harness(next)).toBe("fake");
    expect(daemon.settings.harness(daemon.sessions.create({}).id)).toBe("other");
    // Back to the daemon's own agent: new sessions start there, with nothing to log.
    await daemon.commands.run(next, "/harness main");
    expect(events(daemon, daemon.sessions.create({}).id)).toEqual([]);
  });

  test("a session on a harness gets the user's last settings for it, logged for the panes", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/model fake large");
    const other = daemon.sessions.create({}).id;
    await daemon.commands.run(other, "/harness fake");
    expect(events(daemon, other)).toEqual([
      { type: "setting_changed", harness: "suite", option: "harness", value: "fake" },
      { type: "setting_changed", harness: "fake", option: "model", value: "large" },
    ]);
    // A new session starts on fake, with its model, too.
    expect(events(daemon, daemon.sessions.create({}).id)).toEqual([
      { type: "setting_changed", harness: "suite", option: "harness", value: "fake" },
      { type: "setting_changed", harness: "fake", option: "model", value: "large" },
    ]);
    // A choice made in the session already is not repeated.
    await daemon.commands.run(other, "/harness main");
    await daemon.commands.run(other, "/harness fake");
    const models = events(daemon, other).filter(
      (e) => e.type === "setting_changed" && e.option === "model",
    );
    expect(models.length).toBe(1);
  });

  test("/model and the like default to the harness the session runs on", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/model large");
    await say(daemon, session, "config");
    expect(replies(daemon, session).at(-1)).toContain('"model":"large"');
    expect(daemon.settings.get(session, "fake")).toEqual({ model: "large" });
    await daemon.commands.run(session, "/model");
    const choice = await answer(daemon, session, "choice", "small");
    expect(choice).toMatchObject({ title: "Fake model (now: large)" });
  });
});

describe("a session on an ACP harness", () => {
  test("runs the turns with no agent labels, tells it the connectors and keeps its session id", async () => {
    const { daemon, session, agent } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "hello");
    expect(agent.prompts).toEqual([]);
    const log = events(daemon, session);
    expect(thoughts(daemon, session)[0]).toContain('<app connected="greeter" kind="app" v="1"');
    expect(thoughts(daemon, session)[0]?.endsWith("\nhello")).toBe(true);
    for (const event of log) {
      if (event.type === "agent_message_chunk" || event.type === "agent_thought_chunk") {
        expect(event).not.toHaveProperty("agent");
      }
    }
    expect(replies(daemon, session)).toEqual(["Looking.", "Done: hello"]);
    // The status widgets' "main agent" slot gets these: no agent field.
    expect(log.filter((e) => e.type === "plan")).toHaveLength(1);
    expect(log.find((e) => e.type === "plan")).not.toHaveProperty("agent");
    expect(log.find((e) => e.type === "usage")).toEqual({
      type: "usage",
      contextUsed: 21109,
      contextSize: 1000000,
      cost: 0.25,
    });
    expect(log.filter((e) => e.type === "harness_message")).toEqual([
      { type: "harness_message", harness: "fake", message: { acpSession: "acp-1" } },
    ]);
    expect(log.at(-1)).toEqual({ type: "turn_ended", stopReason: "end_turn" });

    // The next turn only hears what changed since.
    await daemon.connectors.presence({}, { app: "notes", kind: "app", open: true });
    await say(daemon, session, "again");
    const second = thoughts(daemon, session)[1] ?? "";
    expect(second).toContain('<app connected="notes"');
    expect(second).not.toContain("greeter");
    expect(events(daemon, session).filter((e) => e.type === "harness_message")).toHaveLength(1);
  });

  test("its tool calls are prefixed, unlabelled, and get the gateway's text results with refs", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "greet Ada");
    const log = events(daemon, session);
    const call = log.find((e) => e.type === "tool_call");
    expect(call).toMatchObject({ toolCallId: "fake-main:t1" });
    expect(call).not.toHaveProperty("agent");
    // The adapter streams the input afterwards: the update says which tool it is.
    const named = log.find((e) => e.type === "tool_call_update" && e.tool !== undefined);
    expect(named).toMatchObject({ toolCallId: "fake-main:t1", tool: "greeter.greet" });
    const stored = log.find((e) => e.type === "tool_result");
    expect(stored).toMatchObject({ tool: "greeter.greet", rawOutput: { greeted: "Ada" } });
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === "c-fake-main:t1");
    expect(item).toMatchObject({ kind: "tool", state: "done" });
    expect(JSON.stringify(item?.component)).toContain("Ada");
  });

  test("the session's pending events reach it: a guest's turn, and a harness switch's summary", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "/other tidy up");
    // A guest of the same session, labelled, in its own process.
    expect(replies(daemon, session, "other")).toEqual(["Looking.", "Done: tidy up"]);
    expect(replies(daemon, session)).toEqual([]);
    await say(daemon, session, "what happened?");
    const prompt = thoughts(daemon, session).at(-1) ?? "";
    expect(prompt).toContain('<guest agent="other" task="tidy up" stop="end_turn">');
    expect(prompt).toContain("Done: tidy up");
    expect(prompt).not.toContain("conversation-so-far");
  });

  test("cancel and the queue work as for neo", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    daemon.sessions.input(session, "cli", "wait");
    daemon.sessions.input(session, "cli", "after");
    await until(() => replies(daemon, session).length > 0, "the harness to start");
    expect(daemon.sessions.info(session)).toMatchObject({ state: "running", queued: 1 });
    daemon.sessions.cancel(session);
    await daemon.sessions.idle(session);
    const ends = events(daemon, session).filter((e) => e.type === "turn_ended");
    expect(ends).toEqual([
      { type: "turn_ended", stopReason: "cancelled" },
      { type: "turn_ended", stopReason: "end_turn" },
    ]);
    expect(replies(daemon, session).at(-1)).toBe("Done: after");
  });

  test("a restarted daemon resumes its conversation; one that is gone starts anew with the summary", async () => {
    const first = await setup();
    const { daemon: one, session } = first;
    await one.commands.run(session, "/harness fake");
    await say(one, session, "started");
    expect(replies(one, session).at(-1)).toBe("started: new");
    const { dir } = one;
    await one.stop();
    one.stop = () => Promise.resolve();

    const two = await startTestDaemon({ dir, guests: GUESTS });
    cleanups.push(() => two.stop());
    expect(two.settings.harness(session)).toBe("fake");
    await say(two, session, "started");
    expect(replies(two, session).at(-1)).toBe("started: resume");
    expect(thoughts(two, session).at(-1)).not.toContain("conversation-so-far");
    expect(events(two, session).filter((e) => e.type === "harness_message")).toHaveLength(1);
    await two.stop();
    two.stop = () => Promise.resolve();

    // The adapter lost it: a new conversation, told what was said so far.
    const lost = await startTestDaemon({ dir, guests: GUESTS });
    cleanups.push(() => lost.stop());
    lost.store.append(session, {
      type: "harness_message",
      harness: "fake",
      message: { acpSession: "gone-1" },
    });
    await say(lost, session, "started");
    expect(replies(lost, session).at(-1)).toBe("started: new");
    const prompt = thoughts(lost, session).at(-1) ?? "";
    expect(prompt).toContain("<conversation-so-far>");
    expect(prompt).toContain("User: started\nAgent: Looking.started: resume");
    const ids = events(lost, session)
      .filter((e) => e.type === "harness_message")
      .map((e) => e.type === "harness_message" && JSON.stringify(e.message));
    expect(ids).toEqual([
      '{"acpSession":"acp-1"}',
      '{"acpSession":"gone-1"}',
      '{"acpSession":"acp-1"}',
    ]);
  });

  test("a stored harness that is no longer configured falls back to the daemon's agent", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    const { dir } = daemon;
    await daemon.stop();
    daemon.stop = () => Promise.resolve();
    const again = await startTestDaemon({ dir, agent: new FakeAgent("main") });
    cleanups.push(() => again.stop());
    expect(again.sessions.harness(session)).toBe("main");
  });
});

describe("switching harness", () => {
  test("each gets a summary of the turns it missed, newest last", async () => {
    const { daemon, session, agent } = await setup();
    daemon.sessions.input(session, "cli", "first question");
    await agent.started(1);
    expect(agent.prompts[0]?.summary).toBeUndefined();
    agent.release();
    await daemon.sessions.idle(session);

    // To the ACP harness: it is new, so it hears everything so far.
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "second");
    const second = thoughts(daemon, session)[0] ?? "";
    expect(second).toContain("<conversation-so-far>");
    expect(second).toContain("User: first question\nAgent: first question");
    expect(second.indexOf("conversation-so-far")).toBeLessThan(second.indexOf("<app connected"));
    expect(second.endsWith("\nsecond")).toBe(true);

    // Back to neo: only the turn it missed.
    await daemon.commands.run(session, "/harness main");
    daemon.sessions.input(session, "cli", "third");
    await agent.started(2);
    const summary = agent.prompts[1]?.summary ?? "";
    expect(summary).toContain("User: second\nAgent: Done: second");
    expect(summary).not.toContain("first question");
    agent.release();
    await daemon.sessions.idle(session);

    // And to the ACP harness again, which keeps its own conversation: just neo's turn.
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "fourth");
    const fourth = thoughts(daemon, session)[1] ?? "";
    expect(fourth).toContain("User: third\nAgent: third");
    expect(fourth).not.toContain("User: second");
    await say(daemon, session, "fifth");
    expect(thoughts(daemon, session)[2]).not.toContain("conversation-so-far");
  });

  test("a turn that goes to another harness is not handled twice", async () => {
    const { daemon, session, agent } = await setup();
    await daemon.commands.run(session, "/harness fake");
    daemon.sessions.input(session, "cli", "one");
    await until(() => replies(daemon, session).length > 0, "the first turn to start");
    // Chosen while the turn runs: the next turn goes there, this one stays on fake.
    await daemon.commands.run(session, "/harness main");
    await daemon.sessions.idle(session);
    expect(agent.prompts).toEqual([]);
    daemon.sessions.input(session, "cli", "two");
    await agent.started(1);
    expect(agent.prompts[0]?.summary).toContain("User: one\nAgent: Done: one");
    agent.release();
    await daemon.sessions.idle(session);
  });
});

describe("approvals apply to the session's ACP harness", () => {
  test("auto answers without asking; edits allows edits only; ask previews the diff", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/approvals auto");
    await say(daemon, session, "ask execute");
    expect(replies(daemon, session).at(-1)).toBe("answer: yes");
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);

    await daemon.commands.run(session, "/approvals edits");
    await say(daemon, session, "ask edit");
    expect(replies(daemon, session).at(-1)).toBe("answer: yes");
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);
    daemon.sessions.input(session, "cli", "ask execute");
    const execute = await answer(daemon, session, "permission_request", "no");
    expect(execute).toMatchObject({
      agent: "fake",
      title: "ls",
      kind: "execute",
      toolCallId: "fake-main:t2",
    });
    await daemon.sessions.idle(session);
    expect(replies(daemon, session).at(-1)).toBe("answer: no");

    await daemon.commands.run(session, "/approvals ask");
    daemon.sessions.input(session, "cli", "ask edit");
    const edit = await answer(daemon, session, "permission_request", "yes");
    expect(edit).toMatchObject({
      kind: "edit",
      content: [{ type: "diff", path: "/tmp/notes.txt", oldText: "a\n", newText: "b\n" }],
    });
    await daemon.sessions.idle(session);
  });

  test("review mode: the edit is allowed and written, then Keep / Revert; a revert is told next turn", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/approvals review");
    const file = join(daemon.dir, "r.txt");
    await writeFile(file, "before\n");
    daemon.sessions.input(session, "cli", `editfile ${file} after`);
    const card = await answer(daemon, session, "permission_request", "revert");
    expect(card).toMatchObject({
      agent: "fake",
      title: "Edited r.txt — keep it?",
      kind: "edit",
      content: [{ type: "diff", path: file, oldText: "before\n", newText: "after" }],
    });
    await daemon.sessions.idle(session);
    expect(replies(daemon, session).at(-1)).toBe("answer: yes");
    await until(() => events(daemon, session).some((e) => e.type === "edits_reverted"), "revert");
    expect(await readFile(file, "utf8")).toBe("before\n");
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);

    await say(daemon, session, "started");
    expect(thoughts(daemon, session).at(-1)).toContain('<reverted paths="r.txt"/>');
  });

  test("a sensitive read by the ACP harness asks in auto mode; an ordinary one doesn't", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/approvals auto");
    await say(daemon, session, `readfile ${join(daemon.dir, "ok.txt")}`);
    expect(replies(daemon, session).at(-1)).toBe("answer: yes");
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);
    daemon.sessions.input(session, "cli", `readfile ${join(daemon.dir, "secrets.json")}`);
    const request = await answer(daemon, session, "permission_request", "no");
    expect(request).toMatchObject({
      agent: "fake",
      title: "Read secrets.json?",
      kind: "read",
      toolCallId: "fake-main:t5",
    });
    await daemon.sessions.idle(session);
    expect(replies(daemon, session).at(-1)).toBe("answer: no");
  });

  test("edits mode: the review card covers the edits the ACP harness made", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/approvals edits");
    daemon.sessions.input(session, "cli", "edit");
    const card = await answer(daemon, session, "choice", "keep");
    expect(card).toMatchObject({
      title: "Edited 1 file: /tmp/a.txt",
      options: [
        { id: "keep", name: "Keep" },
        { id: "revert", name: "Revert all" },
      ],
    });
    await daemon.sessions.idle(session);
  });
});

describe("delegate from a session on an ACP harness", () => {
  test("the harness's gateway client may delegate (guests may not) and gets text results", async () => {
    const { daemon, session } = await setup();
    const connect = async (headers: Record<string, string>): Promise<Client> => {
      const client = new Client({ name: "test", version: "1" });
      await client.connect(
        new StreamableHTTPClientTransport(new URL(`${daemon.info.url}/mcp`), {
          requestInit: {
            headers: {
              authorization: `Bearer ${daemon.info.token}`,
              "x-suite-session": session,
              ...headers,
            },
          },
        }),
      );
      cleanups.push(() => client.close());
      return client;
    };
    const main = await connect({ "x-suite-text": "1" });
    const names = async (client: Client): Promise<string[]> =>
      (await client.listTools()).tools.map((t) => t.name);
    expect(await names(main)).toContain("delegate");
    const found = await main.callTool({ name: "tools_search", arguments: { query: "greet" } });
    expect(found.structuredContent).toBeUndefined();
    const done = await main.callTool({
      name: "delegate",
      arguments: { agent: "other", task: "x" },
    });
    expect(resultOf(done as { content: unknown[] })).toBe("Done: x");

    const guest = await connect({ "x-suite-text": "1", "x-suite-guest": "fake" });
    expect(await names(guest)).not.toContain("delegate");
    expect(await names(guest)).not.toContain("models");
    expect(await names(guest)).toContain("send_message");
    expect(await names(guest)).toContain("wait");
    const subagent = await connect({ "x-suite-guest": "neo" });
    expect(await names(subagent)).not.toContain("delegate");
    expect(await names(subagent)).not.toContain("models");
    const neo = await subagent.callTool({ name: "tools_search", arguments: { query: "greet" } });
    expect(neo.structuredContent).toBeDefined();
  });

  test("delegating to the session's own harness starts a separate, labelled subagent", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/harness fake");
    await say(daemon, session, "started");
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "started" },
      undefined,
      { session, text: true },
    );
    expect(resultOf(result)).toBe("Looking.started: new");
    // The subagent's updates are labelled, the session's own agent's are not.
    expect(replies(daemon, session)).toEqual(["Looking.", "started: new"]);
    expect(replies(daemon, session, "fake")).toEqual(["Looking.", "started: new"]);
  });

  test("the question names harness, model and effort; the call's own choices win", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/model fake large");
    const ask = async (
      args: Record<string, string>,
      model: string,
      effort: string,
    ): Promise<void> => {
      const call = daemon.gateway.call(
        "delegate",
        { agent: "fake", task: "x", ...args },
        undefined,
        {
          session,
          text: true,
        },
      );
      const request = await answer(daemon, session, "permission_request", "allow");
      expect(request).toMatchObject({
        agent: "fake",
        title: "Start a subagent?",
        details: [
          { key: "Harness", value: "Fake" },
          { key: "Model", value: model },
          { key: "Effort", value: effort },
          { key: "Directory", value: daemon.sessions.info(session)?.cwd },
        ],
      });
      await call;
    };
    await ask({}, "large", "default");
    await ask({ effort: "high" }, "large", "high");
    await ask({ model: "small", effort: "low" }, "small", "low");
  });

  test("model and effort apply to that task only and are not the user's settings", async () => {
    const { daemon, session } = await setup();
    const call = (args: Record<string, string>): Promise<CallToolResult> =>
      daemon.gateway.call("delegate", { agent: "fake", task: "config", ...args }, undefined, {
        session,
        text: true,
      });
    const set = JSON.parse(resultOf(await call({ model: "large", effort: "high" })));
    expect(set.config).toEqual({ model: "large", effort: "high" });
    // Back to what it was for the next task.
    const after = JSON.parse(resultOf(await call({})));
    expect(after.config).toEqual({ model: "small", effort: "low" });
    expect(events(daemon, session).some((e) => e.type === "setting_changed")).toBe(false);
    expect(daemon.settings.get(session, "fake")).toEqual({});

    const unknown = await call({ model: "huge" });
    expect(unknown.isError).toBe(true);
    // Refused before anything was asked or sent, with the names the adapter lists.
    expect(resultOf(unknown)).toBe('Unknown model "huge" for Fake. Valid models: small, large.');
    // Nothing ran: no reply from the subagent for it.
    expect(replies(daemon, session, "fake")).toHaveLength(4);
  });

  test("always allows exactly what was asked for; another model asks again", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    const delegate = (model?: string): Promise<CallToolResult> => {
      const args: Record<string, string> = { agent: "fake", task: "x" };
      if (model !== undefined) args.model = model;
      return daemon.gateway.call("delegate", args, undefined, { session, text: true });
    };
    const first = delegate("small");
    await answer(daemon, session, "permission_request", "always");
    await first;
    const asked = (): number =>
      events(daemon, session).filter((e) => e.type === "permission_request").length;
    expect(asked()).toBe(1);
    await delegate("small");
    expect(asked()).toBe(1);
    const other = delegate("large");
    await answer(daemon, session, "permission_request", "allow");
    await other;
    expect(asked()).toBe(2);
  });

  test("using another harness drops the call's model and effort", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    const call = daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config", model: "large" },
      undefined,
      { session, text: true },
    );
    await answer(daemon, session, "permission_request", "use:other");
    const result = await call;
    expect(resultOf(result)).toContain("(The user chose Other instead.)");
    expect(resultOf(result)).toContain('"config":{}');
  });
});

describe("neo subagents", () => {
  test("delegate to neo: a fresh neo session with the call's model and effort, labelled neo", async () => {
    const model = fakeModel([
      withUsage(reply("Hi from neo."), { input: 10, output: 5 }, 0.01),
      reply("Again."),
    ]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const { daemon, session } = await setup({ neo, gate: "manual" });
    const call = daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "say hi", model: "deepseek/other-model", effort: "high" },
      undefined,
      { session, text: true },
    );
    const request = await answer(daemon, session, "permission_request", "allow");
    expect(request).toMatchObject({
      agent: "main",
      title: "Start a subagent?",
      details: [
        { key: "Harness", value: "neo" },
        { key: "Model", value: "deepseek/other-model" },
        { key: "Effort", value: "high" },
        { key: "Directory", value: daemon.sessions.info(session)?.cwd },
      ],
      content: [{ type: "text", text: "say hi" }],
      options: [
        { id: "allow", name: "Start neo" },
        { id: "always", name: "Allow neo for this session" },
        { id: "use:fake", name: "Use Fake" },
        { id: "use:other", name: "Use Other" },
        { id: "reject", name: "Reject" },
      ],
    });
    expect(resultOf(await call)).toBe("Hi from neo.");
    expect(model.models[0]).toEqual({ provider: "deepseek", id: "other-model" });
    expect(model.reasoning[0]).toBe("high");

    const log = events(daemon, session);
    const chunks = log.filter((e) => e.type === "agent_message_chunk");
    expect(chunks.length).toBeGreaterThan(0);
    for (const chunk of chunks) {
      expect(chunk).toMatchObject({ agent: "neo" });
      expect(chunk.type === "agent_message_chunk" && chunk.messageId.startsWith("neo-")).toBe(true);
    }
    // Its messages are not neo's history of this session.
    expect(log.some((e) => e.type === "harness_message")).toBe(false);
    expect(log.find((e) => e.type === "usage")).toMatchObject({ agent: "neo" });

    // The next task is back on the defaults.
    daemon.settings.set(session, "neo", "model", "deepseek/third-model");
    const again = daemon.gateway.call("delegate", { agent: "neo", task: "again" }, undefined, {
      session,
      text: true,
    });
    await answer(daemon, session, "permission_request", "allow");
    expect(resultOf(await again)).toBe("Again.");
    expect(model.models[1]?.id).toBe(TEST_CONFIG.modelId);
    expect(model.reasoning[1]).not.toBe("high");
  });

  test("an unknown model is the harness's error, and nothing runs", async () => {
    const model = fakeModel([]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const { daemon, session } = await setup({ neo });
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "x", model: "nope/x" },
      undefined,
      { session, text: true },
    );
    expect(result.isError).toBe(true);
    expect(resultOf(result)).toContain('Unknown model provider "nope"');
    expect(model.requests).toHaveLength(0);
  });

  test("a model neo doesn't list is refused with the valid names; its spelling is forgiven", async () => {
    const model = fakeModel([reply("Hi.")]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const { daemon, session } = await setup({ neo });
    const unknown = await daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "x", model: "deepseek v9 turbo" },
      undefined,
      { session, text: true },
    );
    expect(unknown.isError).toBe(true);
    expect(resultOf(unknown)).toStartWith(
      'Unknown model "deepseek v9 turbo" for neo. Valid models: ',
    );
    expect(resultOf(unknown)).toContain("deepseek/deepseek-v4-flash");
    // Nothing was asked or run.
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);
    expect(model.requests).toHaveLength(0);

    const spaced = await daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "x", model: "DeepSeek V4 Flash", effort: "LOW" },
      undefined,
      { session, text: true },
    );
    expect(resultOf(spaced)).toBe("Hi.");
    expect(model.models[0]).toEqual({ provider: "deepseek", id: "deepseek-v4-flash" });
  });

  test("a neo subagent may not delegate", async () => {
    const model = fakeModel([
      reply([fauxToolCall("delegate", { agent: "claude", task: "x" }, { id: "d1" })], "toolUse"),
      reply("Could not."),
    ]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const { daemon, session } = await setup({ neo });
    const result = await daemon.gateway.call("delegate", { agent: "neo", task: "x" }, undefined, {
      session,
      text: true,
    });
    expect(resultOf(result)).toBe("Could not.");
    const update = events(daemon, session).find(
      (e) => e.type === "tool_call_update" && e.toolCallId === "neo:d1" && e.status === "failed",
    );
    expect(JSON.stringify(update)).toContain("Subagents can't delegate.");
  });

  test("without a neo harness, delegate to neo says so", async () => {
    const { daemon, session } = await setup();
    const result = await daemon.gateway.call("delegate", { agent: "neo", task: "x" }, undefined, {
      session,
      text: true,
    });
    expect(resultOf(result)).toBe("No guest named neo (fake, other).");
  });
});

import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { SessionEvent } from "@suite/protocol/types";
import { loadGuestConfigs } from "../src/guests/config.ts";
import { describeCall, type GuestConfig, mcpResultText } from "../src/guests/guests.ts";
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

const greet: AppTool = {
  name: "greet",
  description: "Say hello to someone",
  inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
  handler: (args) => ({
    content: [{ type: "text", text: `Hello, ${String(args.name)}!` }],
    structuredContent: { greeted: args.name },
  }),
};

async function setup(
  options: {
    agent?: FakeAgent;
    guest?: Partial<GuestConfig>;
    guests?: Record<string, GuestConfig>;
    /** How delegate's "start a guest?" question is answered; manual: by the test. */
    gate?: string;
  } = {},
): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({
    agent: options.agent ?? new FakeAgent("main"),
    guests: options.guests ?? { fake: { ...FAKE, ...options.guest } },
  });
  cleanups.push(() => daemon.stop());
  const gate = options.gate ?? "allow";
  if (gate !== "manual") {
    daemon.store.onAppend(({ session, event }) => {
      if (event.type !== "permission_request" || !event.title.startsWith("Hand a task")) return;
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
  const session = daemon.sessions.create({}).id;
  return { daemon, session };
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

describe("delegate", () => {
  test("runs the guest, streams it into the session and returns its last message", async () => {
    const { daemon, session } = await setup();
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "tidy up" },
      undefined,
      session,
    );
    expect(result).toEqual({ content: [{ type: "text", text: "Done: tidy up" }] });
    const log = events(daemon, session);
    const thought = log.find((e) => e.type === "agent_thought_chunk");
    // The guest's first prompt tells it what is open, like the main agent's.
    expect(thought).toMatchObject({ agent: "fake" });
    expect(thought?.type === "agent_thought_chunk" && thought.text).toContain(
      '<app connected="greeter" kind="app" v="1" tools="greet"/>',
    );
    const replies = log.filter((e) => e.type === "agent_message_chunk");
    expect(replies.map((e) => e.type === "agent_message_chunk" && [e.agent, e.text])).toEqual([
      ["fake", "Looking."],
      ["fake", "Done: tidy up"],
    ]);
    // Two messages: the plan update in between started a new one.
    const ids = new Set(replies.map((e) => e.type === "agent_message_chunk" && e.messageId));
    expect(ids.size).toBe(2);

    // The plan replaces the guest's list as a whole; the usage update is the latest numbers.
    expect(log.filter((e) => e.type === "plan")).toEqual([
      {
        type: "plan",
        agent: "fake",
        entries: [
          { content: "Read the code", priority: "high", status: "completed" },
          { content: "Make the change", priority: "medium", status: "in_progress" },
          { content: "Check it", status: "pending" },
        ],
      },
    ]);
    expect(log.filter((e) => e.type === "usage")).toEqual([
      { type: "usage", agent: "fake", contextUsed: 21109, contextSize: 1000000, cost: 0.25 },
    ]);

    // A follow-up continues the same guest session and only says what changed.
    await daemon.gateway.call("delegate", { agent: "fake", task: "again" }, undefined, session);
    const thoughts = events(daemon, session).filter((e) => e.type === "agent_thought_chunk");
    expect(thoughts.at(-1)).toMatchObject({ text: "again" });
  });

  test("usage updates keep the message going and only USD costs are kept", async () => {
    const { daemon, session } = await setup();
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "usage" },
      undefined,
      session,
    );
    expect(result).toEqual({ content: [{ type: "text", text: "Looking.one two three" }] });
    const log = events(daemon, session);
    const ids = new Set(log.map((e) => e.type === "agent_message_chunk" && e.messageId));
    ids.delete(false);
    expect(ids.size).toBe(1);
    expect(log.filter((e) => e.type === "usage")).toEqual([
      { type: "usage", agent: "fake", contextUsed: 100, contextSize: 2000 },
      { type: "usage", agent: "fake", contextUsed: 300, contextSize: 2000 },
    ]);
  });

  test("passes the appended prompt; no settings unless the user chose some", async () => {
    const { daemon, session } = await setup();
    const config = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config" },
      undefined,
      session,
    );
    expect(JSON.parse((config.content[0] as { text: string }).text)).toEqual({
      instructions: "You're working as a guest in a suite of ",
      config: {},
      session: "acp-1",
    });
    const unknown = await daemon.gateway.call(
      "delegate",
      { agent: "nope", task: "x" },
      undefined,
      session,
    );
    expect(unknown.isError).toBe(true);
    expect(JSON.stringify(unknown.content)).toContain("No guest named nope (fake)");
  });

  test("a guest's connector results carry a ref; the view draws them with the app's template", async () => {
    const { daemon, session } = await setup();
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "greet Ada" },
      undefined,
      session,
    );
    expect(result.isError).toBeUndefined();
    const log = events(daemon, session);
    const stored = log.find((e) => e.type === "tool_result");
    expect(stored).toMatchObject({ tool: "greeter.greet", rawOutput: { greeted: "Ada" } });
    if (stored?.type !== "tool_result") throw new Error("no tool_result");
    const { ref } = stored;
    const update = log.findLast((e) => e.type === "tool_call_update");
    expect(JSON.stringify(update)).toContain(`Hello, Ada!\\n[result:${ref}]`);

    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === "c-fake:t1");
    expect(item).toMatchObject({ kind: "tool", label: "fake: greeter.greet", state: "done" });
    expect(JSON.stringify(item?.component)).toContain("Greeted ");
    expect(JSON.stringify(item?.component)).toContain("Ada");
  });

  test("guests don't see delegate and can't call it", async () => {
    const { daemon, session } = await setup();
    const client = new Client({ name: "test", version: "1" });
    await client.connect(
      new StreamableHTTPClientTransport(new URL(`${daemon.info.url}/mcp`), {
        requestInit: {
          headers: {
            authorization: `Bearer ${daemon.info.token}`,
            "x-suite-session": session,
            "x-suite-guest": "fake",
          },
        },
      }),
    );
    cleanups.push(() => client.close());
    const { tools } = await client.listTools();
    expect(tools.map((t) => t.name)).not.toContain("delegate");
    const refused = await client.callTool({
      name: "delegate",
      arguments: { agent: "fake", task: "x" },
    });
    expect(refused.isError).toBe(true);
  });

  test("diffs from a guest's edits announce file changes like the agent's", async () => {
    const { daemon, session } = await setup();
    const changed: string[] = [];
    const fileChanged = daemon.hub.fileChanged.bind(daemon.hub);
    daemon.hub.fileChanged = (payload, except) => {
      changed.push(payload.path);
      fileChanged(payload, except);
    };
    await daemon.gateway.call("delegate", { agent: "fake", task: "edit" }, undefined, session);
    expect(changed).toEqual(["/tmp/a.txt"]);
  });
});

describe("/guest turns", () => {
  test("/fake task runs the guest as the turn and tells the agent what it did", async () => {
    const agent = new FakeAgent();
    const { daemon, session } = await setup({ agent });
    daemon.sessions.input(session, "cli", "/fake fix the parser");
    await daemon.sessions.idle(session);
    const types = events(daemon, session).map((e) => e.type);
    expect(types[0]).toBe("user_message");
    expect(types.slice(-2)).toEqual(["guest_turn", "turn_ended"]);
    expect(events(daemon, session).at(-2)).toEqual({
      type: "guest_turn",
      agent: "fake",
      task: "fix the parser",
      result: "Done: fix the parser",
      stopReason: "end_turn",
    });

    daemon.sessions.input(session, "cli", "what happened?");
    await agent.started(1);
    expect(agent.prompts[0]?.pendingEvents.map((e) => e.type)).toContain("guest_turn");
    agent.release();
    await daemon.sessions.idle(session);
  });

  test("a bare /fake is a usage error; unknown names go to the agent", async () => {
    const agent = new FakeAgent();
    const { daemon, session } = await setup({ agent });
    daemon.sessions.input(session, "cli", "/fake");
    await daemon.sessions.idle(session);
    expect(events(daemon, session).at(-1)).toEqual({
      type: "turn_ended",
      stopReason: "error",
      error: "Usage: /fake <task>",
    });
    daemon.sessions.input(session, "cli", "/nope hi");
    await agent.started(1);
    expect(agent.prompts[0]?.text).toBe("/nope hi");
    agent.release();
    await daemon.sessions.idle(session);
  });

  test("cancel stops the guest", async () => {
    const { daemon, session } = await setup();
    daemon.sessions.input(session, "cli", "/fake wait");
    await until(
      () => events(daemon, session).some((e) => e.type === "agent_message_chunk"),
      "the guest to start",
    );
    daemon.sessions.cancel(session);
    await daemon.sessions.idle(session);
    expect(events(daemon, session).at(-1)).toEqual({ type: "turn_ended", stopReason: "cancelled" });
  });
});

describe("permissions", () => {
  test("a permission request shows as buttons; pressing one answers the guest", async () => {
    const { daemon, session } = await setup();
    daemon.sessions.input(session, "cli", "/fake ask");
    await until(
      () => events(daemon, session).some((e) => e.type === "permission_request"),
      "the permission request",
    );
    const request = events(daemon, session).find((e) => e.type === "permission_request");
    expect(request).toMatchObject({
      agent: "fake",
      title: "Write notes.txt",
      toolCallId: "fake:t2",
    });
    if (request?.type !== "permission_request") throw new Error("no permission_request");
    const { id } = request;
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${id}`);
    expect(item).toMatchObject({
      label: "fake asks: Write notes.txt",
      state: "pending",
      component: {
        type: "row",
        children: [
          { type: "button", label: "Allow", action: "suite.permit", args: { id, option: "yes" } },
          { type: "button", label: "Reject", action: "suite.permit", args: { id, option: "no" } },
        ],
      },
    });

    const press = (option: string): Promise<void> =>
      daemon.actions.run(
        { session, ref: `p-${id}`, action: "suite.permit", args: { id, option } },
        "pad",
      );
    expect(press("maybe")).rejects.toThrow("has no option maybe");
    await daemon.actions.run(
      { session, ref: `p-${id}`, action: "suite.permit", args: { id, option: "yes" } },
      "pad",
    );
    await daemon.sessions.idle(session);
    expect(events(daemon, session)).toContainEqual({
      type: "permission_resolved",
      id,
      option: "yes",
      by: "pad",
    });
    const answered = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${id}`);
    expect(answered).toMatchObject({ state: "done" });
    expect(events(daemon, session).at(-2)).toMatchObject({
      type: "guest_turn",
      result: "answer: yes",
    });
  });

  test("cancelling the turn answers an open request with nothing", async () => {
    const { daemon, session } = await setup();
    daemon.sessions.input(session, "cli", "/fake ask");
    await until(
      () => events(daemon, session).some((e) => e.type === "permission_request"),
      "the permission request",
    );
    daemon.sessions.cancel(session);
    await daemon.sessions.idle(session);
    const resolved = events(daemon, session).find((e) => e.type === "permission_resolved");
    expect(resolved).toBeDefined();
    expect(resolved).not.toHaveProperty("option");
  });

  test("permissions: allow picks the allow option without asking", async () => {
    const { daemon, session } = await setup({ guest: { permissions: "allow" } });
    daemon.sessions.input(session, "cli", "/fake ask");
    await daemon.sessions.idle(session);
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);
    expect(events(daemon, session).at(-2)).toMatchObject({ result: "answer: yes" });
  });
});

describe("config", () => {
  test("defaults, config file and env", () => {
    const dir = join(import.meta.dir, "fixtures-none");
    const base = { HOME: dir, XDG_CONFIG_HOME: dir };
    const all = loadGuestConfigs(base);
    expect(Object.keys(all)).toEqual(["claude", "codex"]);
    expect(all.claude?.command).toEqual(["bunx", "@agentclientprotocol/claude-agent-acp@0.91.0"]);
    const env = loadGuestConfigs({
      ...base,
      SUITE_GUESTS: "claude",
      SUITE_GUEST_PERMISSIONS: "allow",
    });
    expect(Object.keys(env)).toEqual(["claude"]);
    expect(env.claude).toMatchObject({ permissions: "allow" });
    expect(loadGuestConfigs({ ...base, SUITE_GUESTS: "none" })).toEqual({});
    expect(() => loadGuestConfigs({ ...base, SUITE_GUEST_PERMISSIONS: "yolo" })).toThrow(
      "permissions must be ask or allow",
    );
  });
});

describe("adapter shapes", () => {
  test("core tool calls are titled by what they do, for either adapter", () => {
    expect(
      describeCall("mcp__suite__tools_call", { name: "greeter.greet", args: { name: "Ada" } }),
    ).toEqual({ title: "greeter.greet", tool: "greeter.greet" });
    expect(
      describeCall("mcp.suite.tools_call", {
        server: "suite",
        tool: "tools_call",
        arguments: { name: "greeter.greet", args: {} },
      }),
    ).toEqual({ title: "greeter.greet", tool: "greeter.greet" });
    expect(describeCall("mcp.suite.tools_search", { arguments: { query: "greet" } })).toEqual({
      title: "search tools: greet",
    });
    expect(describeCall("mcp__suite__apps_focus", {})).toEqual({ title: "apps_focus" });
    expect(describeCall("Edit a.ts", undefined)).toEqual({ title: "Edit a.ts" });
  });

  test("an MCP result in rawOutput (Codex) becomes the call's text", () => {
    const raw = {
      result: {
        content: [
          { type: "text", text: "Hello, Ada!" },
          { type: "text", text: "[result:3a6548d4]" },
        ],
      },
      error: null,
    };
    expect(mcpResultText(raw)).toEqual([{ type: "text", text: "Hello, Ada!\n[result:3a6548d4]" }]);
    expect(mcpResultText("plain")).toBeUndefined();
  });
});

/** The text of a delegate result. */
function resultOf(result: { content: unknown[] }): string {
  return (result.content[0] as { text: string }).text;
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

function answered(daemon: TestDaemon, session: string, event: SessionEvent): boolean {
  if (event.type !== "choice" && event.type !== "permission_request") return true;
  return events(daemon, session).some(
    (e) => (e.type === "choice_made" || e.type === "permission_resolved") && e.id === event.id,
  );
}

describe("starting a guest is the user's call", () => {
  const twoGuests = { fake: FAKE, other: { ...FAKE, title: "Other" } };

  test("delegate asks first; reject tells the agent", async () => {
    const { daemon, session } = await setup({ gate: "manual", guests: twoGuests });
    const call = daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "tidy up" },
      undefined,
      session,
    );
    const request = await answer(daemon, session, "permission_request", "reject");
    expect(request).toMatchObject({
      agent: "main",
      title: "Hand a task to Fake: tidy up",
      options: [
        { id: "allow", name: "Start Fake", kind: "allow_once" },
        { id: "always", name: "Allow Fake for this session", kind: "allow_always" },
        { id: "use:other", name: "Use Other", kind: "allow_once" },
        { id: "reject", name: "Reject", kind: "reject_once" },
      ],
    });
    const result = await call;
    expect(result.isError).toBe(true);
    expect(resultOf(result)).toContain("The user declined to hand this to Fake");
    expect(events(daemon, session).some((e) => e.type === "agent_message_chunk")).toBe(false);
  });

  test("the user can pick another guest, or allow one for the session", async () => {
    const { daemon, session } = await setup({ gate: "manual", guests: twoGuests });
    const call = daemon.gateway.call("delegate", { agent: "fake", task: "x" }, undefined, session);
    await answer(daemon, session, "permission_request", "use:other");
    const result = await call;
    expect(resultOf(result)).toBe("(The user chose Other instead.)\n\nDone: x");
    expect(events(daemon, session).find((e) => e.type === "agent_message_chunk")).toMatchObject({
      agent: "other",
    });

    const first = daemon.gateway.call("delegate", { agent: "fake", task: "a" }, undefined, session);
    await answer(daemon, session, "permission_request", "always");
    await first;
    const asked = events(daemon, session).filter((e) => e.type === "permission_request").length;
    await daemon.gateway.call("delegate", { agent: "fake", task: "b" }, undefined, session);
    expect(events(daemon, session).filter((e) => e.type === "permission_request")).toHaveLength(
      asked,
    );
  });

  test("the question shows the settings the guest will run with", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    await daemon.commands.run(session, "/model fake large");
    const call = daemon.gateway.call("delegate", { agent: "fake", task: "x" }, undefined, session);
    const request = await answer(daemon, session, "permission_request", "allow");
    expect(request).toMatchObject({ title: "Hand a task to Fake (model large): x" });
    await call;
  });
});

describe("settings", () => {
  test("/model guest value sets it now and for the session; bad values are refused", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/model fake large");
    await daemon.commands.run(session, "/effort fake high");
    expect(events(daemon, session).filter((e) => e.type === "setting_changed")).toEqual([
      { type: "setting_changed", harness: "fake", option: "model", value: "large" },
      { type: "setting_changed", harness: "fake", option: "effort", value: "high" },
    ]);
    const config = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config" },
      undefined,
      session,
    );
    expect(JSON.parse(resultOf(config))).toMatchObject({
      config: { model: "large", effort: "high" },
    });
    expect(daemon.commands.run(session, "/model fake huge")).rejects.toThrow("no model huge");
    expect(daemon.commands.run(session, "/mode fake plan")).rejects.toThrow(
      "Fake has no mode setting",
    );
    const view = daemon.views.get(session).snapshot().items;
    expect(view.some((i) => JSON.stringify(i.component).includes("fake model: large"))).toBe(true);
  });

  test("/model guest without a value offers its models; the pick applies", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/model fake");
    const choice = await answer(daemon, session, "choice", "large");
    expect(choice).toMatchObject({
      title: "Fake model (now: small)",
      options: [
        { id: "small", name: "SMALL", description: "small one" },
        { id: "large", name: "LARGE", description: "large one" },
      ],
    });
    await until(
      () => events(daemon, session).some((e) => e.type === "setting_changed"),
      "the setting",
    );
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => choice.type === "choice" && i.ref === `q-${choice.id}`);
    expect(item).toMatchObject({ state: "done" });
  });

  test("new sessions start with the last choice; the agent gets its settings", async () => {
    const agent = new FakeAgent("main");
    const { daemon, session } = await setup({ agent });
    await daemon.commands.run(session, "/model fake large");
    daemon.settings.set(session, "main", "model", "m2");
    const next = daemon.sessions.create({}).id;
    const config = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config" },
      undefined,
      next,
    );
    expect(JSON.parse(resultOf(config))).toMatchObject({ config: { model: "large" } });
    daemon.sessions.input(next, "cli", "hi");
    await agent.started(1);
    // The guest's work is in the log already, so the agent session is loaded, not created.
    expect(agent.loaded[0]?.settings).toEqual({ model: "m2" });
    agent.release();
    await daemon.sessions.idle(next);
  });

  test("an agent without settings says so", async () => {
    const { daemon, session } = await setup();
    expect(daemon.commands.run(session, "/model")).rejects.toThrow(
      "main has no model setting to pick from",
    );
    expect(daemon.commands.run(session, "/model x")).rejects.toThrow("main has no settings");
    expect(daemon.commands.run(session, "hello")).toBeUndefined();
  });
});

describe("/resume", () => {
  test("lists stored conversations; the pick continues one", async () => {
    const { daemon, session } = await setup();
    await daemon.commands.run(session, "/resume");
    const choice = await answer(daemon, session, "choice", "fake:old-1");
    expect(choice).toMatchObject({
      title: "Resume a conversation",
      options: [{ id: "fake:old-1", name: "Fake: Earlier work", description: "2026-10-01 09:30" }],
    });
    // The resume finishes after the answer; the next task goes to that conversation.
    await Bun.sleep(300);
    const config = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config" },
      undefined,
      session,
    );
    expect(JSON.parse(resultOf(config))).toMatchObject({ session: "old-1" });
    expect(daemon.commands.run(session, "/resume nope")).rejects.toThrow("no guest named nope");
  });
});

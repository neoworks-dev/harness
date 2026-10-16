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
  model: "small",
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
  options: { agent?: FakeAgent; guest?: Partial<GuestConfig> } = {},
): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({
    agent: options.agent,
    guests: { fake: { ...FAKE, ...options.guest } },
  });
  cleanups.push(() => daemon.stop());
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

    // A follow-up continues the same guest session and only says what changed.
    await daemon.gateway.call("delegate", { agent: "fake", task: "again" }, undefined, session);
    const thoughts = events(daemon, session).filter((e) => e.type === "agent_thought_chunk");
    expect(thoughts.at(-1)).toMatchObject({ text: "again" });
  });

  test("passes the appended prompt, model and errors for unknown guests", async () => {
    const { daemon, session } = await setup();
    const config = await daemon.gateway.call(
      "delegate",
      { agent: "fake", task: "config" },
      undefined,
      session,
    );
    expect(JSON.parse((config.content[0] as { text: string }).text)).toEqual({
      instructions: "You're working as a guest in a suite of ",
      config: { model: "small" },
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
      SUITE_CLAUDE_MODEL: "haiku",
      SUITE_GUEST_PERMISSIONS: "allow",
    });
    expect(Object.keys(env)).toEqual(["claude"]);
    expect(env.claude).toMatchObject({ model: "haiku", permissions: "allow" });
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

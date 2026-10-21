import { expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fauxToolCall } from "@mariozechner/pi-ai";
import {
  getOAuthProvider,
  registerOAuthProvider,
  unregisterOAuthProvider,
} from "@mariozechner/pi-ai/oauth";
import type { PlanEntry, SessionEvent } from "@suite/protocol/types";
import type {
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  PromptResult,
} from "../src/agent.ts";
import type { StreamFn } from "../src/loop/index.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import { SYSTEM_PROMPT } from "../src/system-prompt.ts";
import {
  fakeModel,
  hangingModel,
  reply,
  TEST_CONFIG,
  withUsage,
  type FakeModel,
} from "./fake-model.ts";
import { ofType, tempDir } from "./helpers.ts";

async function newSession(model: FakeModel): Promise<{ session: AgentSession; cwd: string }> {
  const cwd = await tempDir();
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  return { session: await agent.newSession({ sessionId: "s1", cwd, mcpServers: [] }), cwd };
}

async function run(
  session: AgentSession,
  text: string,
  pendingEvents: Parameters<AgentSession["prompt"]>[0]["pendingEvents"] = [],
): Promise<{ updates: AgentUpdate[]; result: PromptResult }> {
  const updates: AgentUpdate[] = [];
  const result = await session.prompt({
    text,
    app: "cli",
    pendingEvents,
    onUpdate: (update) => updates.push(update),
  });
  return { updates, result };
}

const harnessMessages = (updates: AgentUpdate[]): unknown[] =>
  ofType(updates, "harness_message").map((u) => u.message);

test("a text reply streams chunks and logs both messages", async () => {
  const { session, cwd } = await newSession(
    fakeModel([
      reply([
        { type: "thinking", thinking: "hmm" },
        { type: "text", text: "Hello there" },
      ]),
    ]),
  );
  const { updates, result } = await run(session, "hi");
  expect(result).toEqual({ stopReason: "end_turn" });
  expect(updates.map((u) => u.type)).toEqual([
    "harness_message",
    "agent_thought_chunk",
    "agent_message_chunk",
    "agent_message_chunk",
    "harness_message",
  ]);
  const chunks = ofType(updates, "agent_message_chunk");
  expect(chunks.map((c) => c.text).join("")).toBe("Hello there");
  const thought = updates.find((u) => u.type === "agent_thought_chunk");
  expect(thought).toMatchObject({ text: "hmm", messageId: chunks[0]?.messageId });
  const [user, assistant] = harnessMessages(updates);
  expect(user).toMatchObject({
    role: "user",
    content: [{ type: "text", text: `<session cwd="${cwd}"/>\nhi` }],
  });
  expect(assistant).toMatchObject({ role: "assistant", stopReason: "stop" });
});

test("pending events are prepended to the text sent to the model", async () => {
  const model = fakeModel([reply("ok")]);
  const { session, cwd } = await newSession(model);
  const { updates } = await run(session, "do it", [
    { type: "app_connected", app: "lsp", kind: "service", v: 1, tools: ["diagnostics"] },
    { type: "focus_changed", app: "browser", context: { url: "https://x" } },
  ]);
  const expected = `<session cwd="${cwd}"/>\n<app connected="lsp" kind="service" v="1" tools="diagnostics"/>\n<focus app="browser" url="https://x"/>\ndo it`;
  expect(harnessMessages(updates)[0]).toMatchObject({ content: [{ text: expected }] });
  expect(model.requests[0]?.[0]).toMatchObject({ role: "user", content: [{ text: expected }] });
});

test("messages from other agents join the conversation after each tool batch, once", async () => {
  const model = fakeModel([
    reply([fauxToolCall("todo_write", { todos: [] }, { id: "t1" })], "toolUse"),
    reply("ok"),
  ]);
  const { session } = await newSession(model);
  const waiting = ['<message from="d_1" agent="claude">\nlook at x\n</message>'];
  const updates: AgentUpdate[] = [];
  const result = await session.prompt({
    text: "go",
    app: "cli",
    pendingEvents: [],
    // Nothing yet when the turn starts; a message by the time the tool has run.
    takeMessages: () => {
      if (!updates.some((u) => u.type === "tool_call_update")) return [];
      return waiting.splice(0);
    },
    onUpdate: (update) => updates.push(update),
  });
  expect(result.stopReason).toBe("end_turn");
  // The second request has it after the tool result; the third (there is none) would not repeat it.
  const second = model.requests[1] ?? [];
  expect(second.map((m) => m.role)).toEqual(["user", "assistant", "toolResult", "user"]);
  expect(JSON.stringify(second.at(-1))).toContain("look at x");
  // It is part of the logged conversation, so a reload keeps it.
  const logged = harnessMessages(updates).map((m) => JSON.stringify(m));
  expect(logged.some((m) => m.includes("look at x"))).toBe(true);
});

test("messages that are there when the turn starts come after the user's text", async () => {
  const model = fakeModel([reply("ok")]);
  const { session } = await newSession(model);
  const waiting = ['<message from="main">\nbegin with y\n</message>'];
  await session.prompt({
    text: "the task",
    app: "delegate",
    pendingEvents: [],
    takeMessages: () => waiting.splice(0),
    onUpdate: () => {},
  });
  const request = model.requests[0] ?? [];
  expect(request.map((m) => m.role)).toEqual(["user", "user"]);
  expect(JSON.stringify(request[0])).toContain("the task");
  expect(JSON.stringify(request[1])).toContain("begin with y");
});

test("the working directory is told once per session, also across a reload", async () => {
  const model = fakeModel([reply("one"), reply("two"), reply("three")]);
  const { session, cwd } = await newSession(model);
  const first = await run(session, "a");
  await run(session, "b");
  const texts = (updates: AgentUpdate[]): string => JSON.stringify(harnessMessages(updates)[0]);
  expect(texts(first.updates)).toContain(`<session cwd=`);
  expect(JSON.stringify(model.requests[1]?.at(-1))).not.toContain("<session cwd=");

  const history: SessionEvent[] = first.updates;
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  const loaded = await agent.loadSession({ sessionId: "s1", cwd, mcpServers: [], history });
  const third = await run(loaded, "c");
  expect(texts(third.updates)).not.toContain("<session cwd=");
});

test("a tool call produces tool_call, updates and a diff in order", async () => {
  const model = fakeModel([
    reply(
      [
        { type: "text", text: "Writing." },
        fauxToolCall("write", { path: "hello.txt", content: "hi" }, { id: "call-1" }),
      ],
      "toolUse",
    ),
    reply("Done."),
  ]);
  const { session, cwd } = await newSession(model);
  const { updates, result } = await run(session, "make a file");

  expect(result.stopReason).toBe("end_turn");
  expect(await readFile(join(cwd, "hello.txt"), "utf8")).toBe("hi");
  expect(
    updates.map((u) =>
      u.type === "harness_message" ? `msg:${(u.message as { role: string }).role}` : u.type,
    ),
  ).toEqual([
    "msg:user",
    "agent_message_chunk",
    "agent_message_chunk",
    "tool_call",
    "msg:assistant",
    "tool_call_update",
    "tool_call_update",
    "msg:toolResult",
    "agent_message_chunk",
    "agent_message_chunk",
    "msg:assistant",
  ]);

  expect(updates.find((u) => u.type === "tool_call")).toEqual({
    type: "tool_call",
    toolCallId: "call-1",
    tool: "write",
    title: "write hello.txt",
    kind: "edit",
    status: "pending",
    rawInput: { path: "hello.txt", content: "hi" },
  });
  const toolUpdates = ofType(updates, "tool_call_update");
  expect(toolUpdates[0]).toEqual({
    type: "tool_call_update",
    toolCallId: "call-1",
    status: "in_progress",
  });
  expect(toolUpdates[1]).toMatchObject({
    toolCallId: "call-1",
    status: "completed",
    content: [
      { type: "text", text: "Created hello.txt (2 bytes)." },
      { type: "diff", path: join(cwd, "hello.txt"), newText: "hi" },
    ],
  });

  const messageIds = ofType(updates, "agent_message_chunk").map((u) => u.messageId);
  expect(new Set(messageIds).size).toBe(2);
  // The second request carries the tool result.
  expect(model.requests[1]?.map((m) => m.role)).toEqual(["user", "assistant", "toolResult"]);
});

test("todo_write emits the plan when the call is made, then answers with a summary", async () => {
  const todos: PlanEntry[] = [
    { content: "read the code", status: "completed" },
    { content: "fix the bug", status: "in_progress" },
    { content: "run the tests", status: "pending" },
  ];
  const model = fakeModel([
    reply([fauxToolCall("todo_write", { todos }, { id: "t1" })], "toolUse"),
    reply("ok"),
  ]);
  const { session } = await newSession(model);
  const { updates } = await run(session, "plan it");
  const kinds = updates.map((u) => u.type).filter((type) => type !== "harness_message");
  expect(kinds).toEqual([
    "tool_call",
    "plan",
    "tool_call_update",
    "tool_call_update",
    "agent_message_chunk",
    "agent_message_chunk",
  ]);
  expect(ofType(updates, "plan")).toEqual([{ type: "plan", entries: todos }]);
  expect(ofType(updates, "tool_call")[0]).toMatchObject({
    tool: "todo_write",
    title: "update todos",
    kind: "think",
  });
  const done = ofType(updates, "tool_call_update").at(-1);
  expect(done).toMatchObject({
    status: "completed",
    content: [{ type: "text", text: "3 todos: 1 done, 1 in progress, 1 pending" }],
    rawOutput: { todos },
  });
});

test("a todo_write call with a malformed list emits no plan", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("todo_write", { todos: [{ content: "x", status: "stuck" }] }, { id: "t1" })],
      "toolUse",
    ),
    reply("ok"),
  ]);
  const { session } = await newSession(model);
  const { updates } = await run(session, "plan it");
  expect(ofType(updates, "plan")).toEqual([]);
  expect(ofType(updates, "tool_call_update").at(-1)?.status).toBe("failed");
});

test("a failing tool is reported as failed and the turn continues", async () => {
  const model = fakeModel([
    reply(
      [
        fauxToolCall(
          "bash",
          { command: "echo oops; exit 2\nsecond line", description: "Fail on purpose" },
          { id: "b1" },
        ),
      ],
      "toolUse",
    ),
    reply("It failed."),
  ]);
  const { session } = await newSession(model);
  const { updates, result } = await run(session, "run it");
  expect(result.stopReason).toBe("end_turn");
  expect(updates.find((u) => u.type === "tool_call")).toMatchObject({
    title: "Fail on purpose",
    kind: "execute",
  });
  const last = ofType(updates, "tool_call_update").at(-1);
  expect(last?.status).toBe("failed");
  expect(JSON.stringify(last?.content)).toContain("exited with code 2");
});

test("declining a command with no note stops the turn: the model doesn't answer it", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("bash", { command: "touch x", description: "Create x" }, { id: "b1" })],
      "toolUse",
    ),
    reply("I see you declined."),
  ]);
  const cwd = await tempDir();
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  const session = await agent.newSession({
    sessionId: "s1",
    cwd,
    mcpServers: [],
    requestPermission: async () => ({ allowed: false }),
  });
  const { result } = await run(session, "make x");
  expect(result.stopReason).toBe("cancelled");
  expect(model.requests).toHaveLength(1);
  expect(existsSync(join(cwd, "x"))).toBe(false);
});

test("declining with a note goes on: the model gets the note", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("bash", { command: "touch x", description: "Create x" }, { id: "b1" })],
      "toolUse",
    ),
    reply("Making y instead."),
  ]);
  const cwd = await tempDir();
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  const session = await agent.newSession({
    sessionId: "s1",
    cwd,
    mcpServers: [],
    requestPermission: async () => ({ allowed: false, note: "make y instead" }),
  });
  const { result } = await run(session, "make x");
  expect(result.stopReason).toBe("end_turn");
  expect(model.requests).toHaveLength(2);
  expect(JSON.stringify(model.requests[1])).toContain("make y instead");
});

test("every neo message is logged as a harness_message", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("bash", { command: "echo hi", description: "test" }, { id: "b1" })],
      "toolUse",
    ),
    reply("done"),
  ]);
  const { session } = await newSession(model);
  const { updates } = await run(session, "go");
  const logged = harnessMessages(updates);
  expect(logged.map((m) => (m as { role: string }).role)).toEqual([
    "user",
    "assistant",
    "toolResult",
    "assistant",
  ]);
  for (const update of updates) {
    if (update.type === "harness_message") expect(update.harness).toBe("neo");
  }
});

test("the system prompt and tools are sent unchanged on every request", async () => {
  const seen: { prompt?: string; tools: string[] }[] = [];
  const spy =
    (): ((context: {
      systemPrompt?: string;
      tools?: { name: string }[];
    }) => ReturnType<typeof reply>) =>
    (context) => {
      seen.push({ prompt: context.systemPrompt, tools: (context.tools ?? []).map((t) => t.name) });
      return reply("ok");
    };
  const { session } = await newSession(fakeModel([spy(), spy()]));
  await run(session, "one");
  await run(session, "two", [{ type: "app_disconnected", app: "x" }]);
  expect(seen[0]).toEqual({
    prompt: SYSTEM_PROMPT,
    tools: [
      "bash",
      "read",
      "write",
      "edit",
      "todo_write",
      "skill",
      "web_search",
      "web_fetch",
      "tools_search",
      "tools_call",
      "apps_launch",
      "apps_focus",
      "delegate",
      "models",
      "send_message",
      "wait",
      "ui_library",
    ],
  });
  expect(seen[1]).toEqual(seen[0]);
});

test("a session lists its skills in the system prompt, and the skill tool loads one", async () => {
  const root = await tempDir();
  const bundled = join(root, "bundled");
  await mkdir(join(bundled, "demo"), { recursive: true });
  await writeFile(
    join(bundled, "demo", "SKILL.md"),
    "---\nname: demo\ndescription: Use when demoing\n---\n\nStep one.\nStep two.\n",
  );
  const cwd = join(root, "project");
  await mkdir(join(cwd, ".suite", "skills", "local"), { recursive: true });
  await writeFile(
    join(cwd, ".suite", "skills", "local", "SKILL.md"),
    "---\nname: local\ndescription: Project rules\n---\nUse tabs.",
  );
  const prompts: (string | undefined)[] = [];
  const model = fakeModel([
    (context) => {
      prompts.push(context.systemPrompt);
      return reply(
        [{ type: "toolCall", id: "c1", name: "skill", arguments: { name: "demo" } }],
        "toolUse",
      );
    },
    (context) => {
      const last = context.messages.at(-1);
      prompts.push(JSON.stringify(last));
      return reply("loaded");
    },
  ]);
  const config = { ...TEST_CONFIG, skills: { bundled, user: join(root, "none") } };
  const agent = createNeoAgent({ config, streamFn: model.streamFn });
  const session = await agent.newSession({ sessionId: "s1", cwd, mcpServers: [] });
  const { updates } = await run(session, "demo it");
  expect(prompts[0]?.startsWith(SYSTEM_PROMPT)).toBe(true);
  expect(prompts[0]).toContain("- demo: Use when demoing\n- local: Project rules");
  expect(prompts[1]).toContain("Step one.\\nStep two.");
  const call = updates.find((u) => u.type === "tool_call");
  expect(call).toMatchObject({ tool: "skill", title: "skill demo", kind: "other" });
  const done = ofType(updates, "tool_call_update").find((u) => u.status === "completed");
  expect(done?.content).toEqual([{ type: "text", text: "2 lines" }]);
});

test("usage follows each assistant message with running totals", async () => {
  const model = fakeModel([
    withUsage(
      reply(
        [fauxToolCall("bash", { command: "true", description: "test" }, { id: "b1" })],
        "toolUse",
      ),
      { input: 100, output: 20, cacheRead: 50 },
      0.01,
    ),
    withUsage(reply("done"), { input: 30, output: 5, cacheRead: 150, cacheWrite: 10 }, 0.02),
  ]);
  const { session } = await newSession(model);
  const { updates } = await run(session, "go");
  const usage = ofType(updates, "usage");
  expect(usage).toHaveLength(2);
  expect(usage[0]).toEqual({
    type: "usage",
    contextUsed: 170,
    contextSize: expect.any(Number),
    cost: 0.01,
    inputTokens: 150,
    outputTokens: 20,
  });
  expect(usage[0]?.contextSize).toBeGreaterThan(0);
  expect(usage[1]).toMatchObject({
    contextUsed: 195,
    cost: expect.closeTo(0.03, 10),
    inputTokens: 340,
    outputTokens: 25,
  });
  // The usage update comes right after the message it describes.
  const types = updates.map((u) => u.type);
  expect(types.at(-1)).toBe("usage");
  expect(types.at(-2)).toBe("harness_message");
});

test("usage keeps adding up after the session is loaded from the log", async () => {
  const model = fakeModel([
    withUsage(reply("one"), { input: 100, output: 10 }, 0.5),
    withUsage(reply("two"), { input: 200, output: 20 }, 0.25),
  ]);
  const { session, cwd } = await newSession(model);
  const first = await run(session, "a");
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  const loaded = await agent.loadSession({
    sessionId: "s1",
    cwd,
    mcpServers: [],
    history: first.updates,
  });
  const second = await run(loaded, "b");
  expect(ofType(second.updates, "usage")).toEqual([
    {
      type: "usage",
      contextUsed: 220,
      contextSize: expect.any(Number),
      cost: 0.75,
      inputTokens: 300,
      outputTokens: 30,
    },
  ]);
});

test("replies without usage emit no usage update", async () => {
  const { session } = await newSession(fakeModel([reply("hi")]));
  const { updates } = await run(session, "x");
  expect(ofType(updates, "usage")).toEqual([]);
});

test("stop reasons map to the contract", async () => {
  const length = await newSession(fakeModel([reply("cut", "length")]));
  expect((await run(length.session, "x")).result).toEqual({ stopReason: "max_tokens" });

  const failing = await newSession(fakeModel([reply("", "error", "provider exploded")]));
  expect((await run(failing.session, "x")).result).toEqual({
    stopReason: "error",
    error: "provider exploded",
  });
});

test("a throwing model resolves with an error instead of rejecting", async () => {
  const { session } = await newSession(fakeModel([]));
  const { result } = await run(session, "x");
  expect(result.stopReason).toBe("error");
  expect(result.error).toContain("unexpected model request");
});

test("a missing API key resolves with an error naming the env var, without calling the model", async () => {
  const model = fakeModel([reply("never")]);
  const agent = createNeoAgent({
    config: { ...TEST_CONFIG, apiKey: undefined },
    streamFn: model.streamFn,
  });
  const saved = process.env.DEEPSEEK_API_KEY;
  delete process.env.DEEPSEEK_API_KEY;
  try {
    const session = await agent.newSession({
      sessionId: "s",
      cwd: await tempDir(),
      mcpServers: [],
    });
    const { updates, result } = await run(session, "x");
    expect(result.stopReason).toBe("error");
    expect(result.error).toContain("DEEPSEEK_API_KEY");
    expect(updates).toEqual([]);
    expect(model.requests).toEqual([]);
  } finally {
    if (saved !== undefined) process.env.DEEPSEEK_API_KEY = saved;
  }
});

test("cancel aborts the running prompt and it resolves cancelled", async () => {
  const model = hangingModel();
  const { session } = await newSession(model);
  session.cancel(); // idle: no-op
  const running = run(session, "wait");
  await Bun.sleep(20);
  session.cancel();
  const { result } = await running;
  expect(result).toEqual({ stopReason: "cancelled" });

  // The session is usable afterwards.
  const next = fakeModel([reply("back")]);
  const { session: other } = await newSession(next);
  expect((await run(other, "x")).result.stopReason).toBe("end_turn");
});

test("cancel during a bash command stops it and ends the turn cancelled", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("bash", { command: "sleep 30", description: "test" }, { id: "b1" })],
      "toolUse",
    ),
    (_context, signal) => reply("", signal?.aborted ? "aborted" : "stop", "aborted"),
  ]);
  const { session } = await newSession(model);
  const running = run(session, "sleep");
  await Bun.sleep(150);
  session.cancel();
  const started = Date.now();
  const { result } = await running;
  expect(result.stopReason).toBe("cancelled");
  expect(Date.now() - started).toBeLessThan(3000);
});

async function sessionWith(
  model: FakeModel,
  settings?: Parameters<ReturnType<typeof createNeoAgent>["newSession"]>[0]["settings"],
  config = TEST_CONFIG,
): Promise<AgentSession> {
  const agent = createNeoAgent({ config, streamFn: model.streamFn });
  return agent.newSession({ sessionId: "s1", cwd: await tempDir(), mcpServers: [], settings });
}

const optionOf = (session: AgentSession, category: string): ConfigOption | undefined =>
  session.configOptions?.().find((option) => option.category === category);

function setOption(session: AgentSession, category: ConfigCategory, value: string): Promise<void> {
  if (!session.setConfigOption) throw new Error("the session has no setConfigOption");
  return session.setConfigOption(category, value);
}

test("settings are applied when the session is created", async () => {
  const model = fakeModel([reply("ok")]);
  const session = await sessionWith(model, {
    model: "deepseek/deepseek-v4-pro",
    thought_level: "high",
  });
  expect(optionOf(session, "model")?.current).toBe("deepseek/deepseek-v4-pro");
  expect(optionOf(session, "thought_level")?.current).toBe("high");
  await run(session, "hi");
  expect(model.models[0]).toEqual({ provider: "deepseek", id: "deepseek-v4-pro" });
});

test("a bare model id means the configured provider", async () => {
  const model = fakeModel([reply("ok")]);
  const session = await sessionWith(model, { model: "deepseek-v4-pro" });
  expect(optionOf(session, "model")?.current).toBe("deepseek/deepseek-v4-pro");
});

test("invalid stored settings fall back to the defaults", async () => {
  const warnings: string[] = [];
  const logger = { warn: (message: string) => warnings.push(message) } as never;
  const agent = createNeoAgent({ config: TEST_CONFIG, logger });
  for (const settings of [
    { model: "nope/model", thought_level: "ludicrous" },
    { model: "deepseek/" },
  ]) {
    const session = await agent.newSession({
      sessionId: "s1",
      cwd: await tempDir(),
      mcpServers: [],
      settings,
    });
    expect(optionOf(session, "model")?.current).toBe("deepseek/deepseek-v4-flash");
    expect(optionOf(session, "thought_level")?.current).toBe("off");
  }
  expect(warnings.length).toBe(3);
});

test("configOptions lists the provider's models and the thinking levels", async () => {
  const session = await sessionWith(fakeModel([]));
  const model = optionOf(session, "model");
  expect(model?.current).toBe("deepseek/deepseek-v4-flash");
  expect(model?.values).toContainEqual({
    value: "deepseek/deepseek-v4-pro",
    name: expect.any(String),
  });
  expect(optionOf(session, "thought_level")).toEqual({
    category: "thought_level",
    current: "off",
    values: ["off", "minimal", "low", "medium", "high", "xhigh"].map((v) => ({
      value: v,
      name: v,
    })),
  });
  expect(optionOf(session, "mode")).toBeUndefined();
});

test("configOptions includes a current model the registry does not know", async () => {
  const session = await sessionWith(fakeModel([]), { model: "deepseek/deepseek-v9" });
  const model = optionOf(session, "model");
  expect(model?.current).toBe("deepseek/deepseek-v9");
  expect(model?.values.map((v) => v.value)).toContain("deepseek/deepseek-v9");
});

test("setConfigOption changes the model and thinking level for the next prompt", async () => {
  const model = fakeModel([reply("one"), reply("two")]);
  const session = await sessionWith(model);
  await run(session, "a");
  await setOption(session, "model", "deepseek/deepseek-v4-pro");
  await setOption(session, "thought_level", "medium");
  expect(optionOf(session, "model")?.current).toBe("deepseek/deepseek-v4-pro");
  expect(optionOf(session, "thought_level")?.current).toBe("medium");
  await run(session, "b");
  expect(model.models.map((m) => m.id)).toEqual(["deepseek-v4-flash", "deepseek-v4-pro"]);
  expect(model.reasoning[1]).toBe("medium");
});

test("the API key follows the model's provider", async () => {
  const config = { ...TEST_CONFIG, apiKey: undefined, provider: "openai", modelId: "gpt-4o" };
  const saved = { deepseek: process.env.DEEPSEEK_API_KEY, openai: process.env.OPENAI_API_KEY };
  process.env.DEEPSEEK_API_KEY = "deepseek-key";
  delete process.env.OPENAI_API_KEY;
  try {
    const model = fakeModel([reply("ok")]);
    const session = await sessionWith(model, undefined, config);
    await setOption(session, "model", "deepseek/deepseek-v4-pro");
    const { result } = await run(session, "hi");
    expect(result).toEqual({ stopReason: "end_turn" });
    await setOption(session, "model", "gpt-4o");
    const missing = await run(session, "hi");
    expect(missing.result.error).toContain('"openai"');
  } finally {
    if (saved.deepseek === undefined) delete process.env.DEEPSEEK_API_KEY;
    else process.env.DEEPSEEK_API_KEY = saved.deepseek;
    if (saved.openai !== undefined) process.env.OPENAI_API_KEY = saved.openai;
  }
});

const COPILOT_ENV = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

async function withAccount<T>(
  provider: string,
  account: unknown,
  body: (config: typeof TEST_CONFIG) => Promise<T>,
): Promise<T> {
  const authPath = join(await tempDir(), "auth.json");
  await writeFile(authPath, JSON.stringify({ [provider]: account }));
  const saved = COPILOT_ENV.map((name) => process.env[name]);
  for (const name of COPILOT_ENV) delete process.env[name];
  try {
    return await body({ ...TEST_CONFIG, apiKey: undefined, authPath, provider, modelId: "x" });
  } finally {
    COPILOT_ENV.forEach((name, i) => {
      const value = saved[i];
      if (value !== undefined) process.env[name] = value;
    });
  }
}

test("a Copilot login is sent as its token, to the base URL its token names", async () => {
  const token = "tid=1;exp=2;proxy-ep=proxy.business.githubcopilot.com";
  const account = { type: "oauth", access: token, refresh: "r", expires: Date.now() + 3_600_000 };
  await withAccount("github-copilot", account, async (config) => {
    const seen: { baseUrl: string; apiKey: string | undefined }[] = [];
    const inner = fakeModel([reply("ok")]);
    const streamFn: StreamFn = (model, context, options) => {
      seen.push({ baseUrl: model.baseUrl, apiKey: options?.apiKey });
      return inner.streamFn(model, context, options);
    };
    const agent = createNeoAgent({ config, streamFn });
    const session = await agent.newSession({
      sessionId: "s",
      cwd: await tempDir(),
      mcpServers: [],
    });
    expect((await run(session, "hi")).result).toEqual({ stopReason: "end_turn" });
    expect(seen).toEqual([{ baseUrl: "https://api.business.githubcopilot.com", apiKey: token }]);
  });
});

test("a login that can't be refreshed ends the turn with an error, without calling the model", async () => {
  const original = getOAuthProvider("openai-codex");
  if (!original) throw new Error("pi-ai has no openai-codex provider");
  registerOAuthProvider({
    ...original,
    refreshToken: () => Promise.reject(new Error("invalid_grant")),
  });
  try {
    const account = { type: "oauth", access: "a", refresh: "r", expires: Date.now() - 1000 };
    await withAccount("openai-codex", account, async (config) => {
      const model = fakeModel([reply("never")]);
      const agent = createNeoAgent({ config, streamFn: model.streamFn });
      const session = await agent.newSession({
        sessionId: "s",
        cwd: await tempDir(),
        mcpServers: [],
      });
      const { result } = await run(session, "hi");
      expect(result.stopReason).toBe("error");
      expect(result.error).toContain("Log in again");
      expect(model.requests).toEqual([]);
    });
  } finally {
    unregisterOAuthProvider("openai-codex");
  }
});

async function rejection(promise: Promise<void>): Promise<string> {
  try {
    await promise;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return "";
}

test("setConfigOption rejects values it can't use", async () => {
  const session = await sessionWith(fakeModel([]));
  expect(await rejection(setOption(session, "thought_level", "max"))).toMatch(
    /Unknown thinking level "max".*off, minimal/,
  );
  expect(await rejection(setOption(session, "model", "nope/x"))).toContain(
    'Unknown model provider "nope"',
  );
  expect(await rejection(setOption(session, "mode", "plan"))).toContain("neo has no mode setting");
  expect(optionOf(session, "model")?.current).toBe("deepseek/deepseek-v4-flash");
  expect(optionOf(session, "thought_level")?.current).toBe("off");
});

test("reading a picture shows it: the result carries the file as image content", async () => {
  const model = fakeModel([
    reply([fauxToolCall("read", { path: "dot.png" }, { id: "call-1" })], "toolUse"),
    reply("A dot."),
  ]);
  const { session, cwd } = await newSession(model);
  // A 1x1 png.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
    "base64",
  );
  await Bun.write(join(cwd, "dot.png"), png);
  const { updates } = await run(session, "what is in dot.png?");
  const done = ofType(updates, "tool_call_update").find((u) => u.status === "completed");
  expect(done?.content).toEqual([
    { type: "text", text: "Read image dot.png (image/png, 70 bytes)." },
    { type: "image", src: join(cwd, "dot.png"), mimeType: "image/png" },
  ]);
});

test("generateText asks the configured model, or the one given, with the system prompt and no tools", async () => {
  const model = fakeModel([
    reply("one"),
    reply([
      { type: "text", text: "tw" },
      { type: "text", text: "o" },
    ]),
  ]);
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  expect(await agent.generateText?.({ system: "Be brief.", prompt: "Hello" })).toBe("one");
  expect(model.models[0]).toEqual({ provider: "deepseek", id: "deepseek-v4-flash" });
  expect(model.requests[0]).toMatchObject([{ role: "user", content: "Hello" }]);
  expect(model.reasoning[0]).toBeUndefined();
  const given = { system: "s", prompt: "p", model: "deepseek/deepseek-v4" };
  expect(await agent.generateText?.(given)).toBe("two");
  expect(model.models[1]).toEqual({ provider: "deepseek", id: "deepseek-v4" });
});

test("generateText rejects with the model's error, and for an unknown provider or no key", async () => {
  const model = fakeModel([reply([], "error", "rate limited")]);
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  const asking = async (target: typeof agent, setting?: string): Promise<string> => {
    try {
      await target.generateText?.({ system: "s", prompt: "p", model: setting });
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
    return "";
  };
  expect(await asking(agent)).toContain("rate limited");
  expect(await asking(agent, "nowhere/x")).toContain('Unknown model provider "nowhere"');
  const keyless = createNeoAgent({
    config: { ...TEST_CONFIG, apiKey: undefined },
    streamFn: fakeModel([]).streamFn,
  });
  const saved = process.env.DEEPSEEK_API_KEY;
  delete process.env.DEEPSEEK_API_KEY;
  try {
    expect(await asking(keyless)).toContain('No API key or login for provider "deepseek"');
  } finally {
    if (saved !== undefined) process.env.DEEPSEEK_API_KEY = saved;
  }
});

import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fauxToolCall } from "@mariozechner/pi-ai";
import type { SessionEvent } from "@suite/protocol/types";
import type { AgentSession, AgentUpdate, PromptResult } from "../src/agent.ts";
import { createPiAgent } from "../src/pi-agent.ts";
import { SYSTEM_PROMPT } from "../src/system-prompt.ts";
import { fakeModel, hangingModel, reply, TEST_CONFIG, type FakeModel } from "./fake-model.ts";
import { ofType, tempDir } from "./helpers.ts";

async function newSession(model: FakeModel): Promise<{ session: AgentSession; cwd: string }> {
  const cwd = await tempDir();
  const agent = createPiAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
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

test("the working directory is told once per session, also across a reload", async () => {
  const model = fakeModel([reply("one"), reply("two"), reply("three")]);
  const { session, cwd } = await newSession(model);
  const first = await run(session, "a");
  await run(session, "b");
  const texts = (updates: AgentUpdate[]): string => JSON.stringify(harnessMessages(updates)[0]);
  expect(texts(first.updates)).toContain(`<session cwd=`);
  expect(JSON.stringify(model.requests[1]?.at(-1))).not.toContain("<session cwd=");

  const history: SessionEvent[] = first.updates;
  const agent = createPiAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
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

test("a failing tool is reported as failed and the turn continues", async () => {
  const model = fakeModel([
    reply(
      [fauxToolCall("bash", { command: "echo oops; exit 2\nsecond line" }, { id: "b1" })],
      "toolUse",
    ),
    reply("It failed."),
  ]);
  const { session } = await newSession(model);
  const { updates, result } = await run(session, "run it");
  expect(result.stopReason).toBe("end_turn");
  expect(updates.find((u) => u.type === "tool_call")).toMatchObject({
    title: "bash: echo oops; exit 2",
    kind: "execute",
  });
  const last = ofType(updates, "tool_call_update").at(-1);
  expect(last?.status).toBe("failed");
  expect(JSON.stringify(last?.content)).toContain("exited with code 2");
});

test("every pi message is logged as a harness_message", async () => {
  const model = fakeModel([
    reply([fauxToolCall("bash", { command: "echo hi" }, { id: "b1" })], "toolUse"),
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
    if (update.type === "harness_message") expect(update.harness).toBe("pi");
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
      "tools_search",
      "tools_call",
      "apps_launch",
      "apps_focus",
    ],
  });
  expect(seen[1]).toEqual(seen[0]);
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
  const agent = createPiAgent({
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
    reply([fauxToolCall("bash", { command: "sleep 30" }, { id: "b1" })], "toolUse"),
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

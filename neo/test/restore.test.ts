import { expect, test } from "bun:test";
import { fauxToolCall, type Message } from "@mariozechner/pi-ai";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import type { AgentSession, AgentUpdate } from "../src/agent.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import { INTERRUPTED_RESULT, repairMessages, restoreMessages } from "../src/restore.ts";
import { fakeModel, reply, TEST_CONFIG, type FakeModel } from "./fake-model.ts";
import { tempDir } from "./helpers.ts";

function user(text: string): Message {
  return { role: "user", content: [{ type: "text", text }], timestamp: 1 };
}

function toolResult(id: string, name = "bash", text = "ok"): Message {
  return {
    role: "toolResult",
    toolCallId: id,
    toolName: name,
    content: [{ type: "text", text }],
    isError: false,
    timestamp: 3,
  };
}

function call(id: string): ReturnType<typeof fauxToolCall> {
  return fauxToolCall("bash", { command: `echo ${id}`, description: "test" }, { id });
}

function logged(messages: Message[]): SessionEvent[] {
  return messages.flatMap((message): SessionEvent[] => [
    { type: "user_message", app: "cli", text: "ignored" },
    { type: "harness_message", harness: "neo", message },
    { type: "harness_message", harness: "other", message: user("not ours") },
  ]);
}

/** What providers require: every tool call answered right after its assistant message, once. */
function expectValid(messages: Message[]): void {
  const open = new Set<string>();
  for (const message of messages) {
    if (message.role === "toolResult") {
      expect(open.delete(message.toolCallId)).toBe(true);
      continue;
    }
    expect(open.size).toBe(0);
    if (message.role === "assistant") {
      expect(message.stopReason === "error" || message.stopReason === "aborted").toBe(false);
      for (const block of message.content) if (block.type === "toolCall") open.add(block.id);
    }
  }
  expect(open.size).toBe(0);
}

async function load(history: SessionEvent[], model: FakeModel): Promise<AgentSession> {
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  return agent.loadSession({ sessionId: "s1", cwd: await tempDir(), history, mcpServers: [] });
}

async function prompt(session: AgentSession, text: string): Promise<AgentUpdate[]> {
  const updates: AgentUpdate[] = [];
  await session.prompt({ text, app: "cli", pendingEvents: [], onUpdate: (u) => updates.push(u) });
  return updates;
}

test("restores only the neo messages, in order", () => {
  const assistant = reply("hello");
  const messages = restoreMessages(logged([user("a"), assistant, user("b")]));
  expect(messages).toEqual([user("a"), assistant, user("b")]);
});

test("restores only what came after the context was last cleared", () => {
  const history = [
    ...logged([user("a"), reply("old")]),
    { type: "context_cleared" } as const,
    ...logged([user("b")]),
  ];
  expect(restoreMessages(history)).toEqual([user("b")]);
});

test("a completed session continues exactly where it left off", async () => {
  // First run: a real session produces the log.
  const first = fakeModel([reply([call("c1")], "toolUse"), reply("all done")]);
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: first.streamFn });
  const session = await agent.newSession({ sessionId: "s1", cwd: await tempDir(), mcpServers: [] });
  const history: SessionEvent[] = [];
  const updates = await prompt(session, "first");
  for (const update of updates) history.push(update);

  const second = fakeModel([reply("second answer")]);
  const loaded = await load(history, second);
  await prompt(loaded, "second");

  const sent = second.requests[0] ?? [];
  expect(sent.map((m) => m.role)).toEqual(["user", "assistant", "toolResult", "assistant", "user"]);
  expect(sent.slice(0, 4)).toEqual(restoreMessages(history));
  expect(sent.slice(0, 3)).toEqual(first.requests[1] ?? []);
  expectValid(sent);
});

test("a tool call without a result gets an interrupted result right after it", async () => {
  const assistant = reply([call("a"), call("b")], "toolUse");
  const history = logged([user("go"), assistant, toolResult("a")]);
  const model = fakeModel([reply("recovered")]);
  const updates = await prompt(await load(history, model), "continue");

  const sent = model.requests[0] ?? [];
  expect(sent.map((m) => m.role)).toEqual([
    "user",
    "assistant",
    "toolResult",
    "toolResult",
    "user",
  ]);
  expect(sent[2]).toMatchObject({ toolCallId: "a", content: [{ text: "ok" }] });
  expect(sent[3]).toMatchObject({
    toolCallId: "b",
    isError: true,
    content: [{ text: INTERRUPTED_RESULT }],
  });
  expectValid(sent);
  expect(updates.at(-1)).toMatchObject({ type: "harness_message", message: { role: "assistant" } });
});

test("a crash before any result repairs every call; the history may end on a tool call", () => {
  const repaired = repairMessages([user("go"), reply([call("a"), call("b")], "toolUse")]);
  expect(repaired.map((m) => m.role)).toEqual(["user", "assistant", "toolResult", "toolResult"]);
  expectValid(repaired);
});

test("results are put in call order and strays are dropped", () => {
  const repaired = repairMessages([
    user("go"),
    reply([call("a"), call("b")], "toolUse"),
    toolResult("b"),
    toolResult("zzz"),
    toolResult("a"),
    user("next"),
    toolResult("late"),
  ]);
  expect(repaired.filter((m) => m.role === "toolResult").map((m) => m.toolCallId)).toEqual([
    "a",
    "b",
  ]);
  expectValid(repaired);
});

test("errored and aborted assistant messages are dropped", async () => {
  const history = logged([
    user("one"),
    reply("", "error", "boom"),
    user("two"),
    reply([{ type: "text", text: "partial" }, call("x")], "aborted"),
    user("three"),
    reply("fine"),
  ]);
  const model = fakeModel([reply("ok")]);
  await prompt(await load(history, model), "four");
  const sent = model.requests[0] ?? [];
  expect(sent.map((m) => m.role)).toEqual(["user", "user", "user", "assistant", "user"]);
  expectValid(sent);
});

test("repair is deterministic", () => {
  const history = logged([user("go"), reply([call("a")], "toolUse")]);
  expect(restoreMessages(history)).toEqual(restoreMessages(history));
  expect(JSON.stringify(restoreMessages(history))).toBe(JSON.stringify(restoreMessages(history)));
});

test("junk harness messages are skipped with a warning", () => {
  const warnings: string[] = [];
  const history: SessionEvent[] = [
    { type: "harness_message", harness: "neo", message: { nope: true } },
    { type: "harness_message", harness: "neo", message: user("kept") },
  ];
  const logger = { warn: (m: string) => warnings.push(m), error: () => {} };
  expect(restoreMessages(history, logger)).toEqual([user("kept")]);
  expect(warnings).toHaveLength(1);
});

import { expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fauxToolCall } from "@mariozechner/pi-ai";
import type {
  AgentSession,
  AgentUpdate,
  PermissionAsk,
  PromptResult,
  RequestPermission,
} from "../src/agent.ts";
import type { HarnessConfig } from "../src/config.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import { fakeModel, reply, TEST_CONFIG, type FakeModel } from "./fake-model.ts";
import { ofType, tempDir } from "./helpers.ts";

const HAVE_PYTHON = Bun.which("python3") !== null;
const HAVE_RALPH_TOOLS = Bun.which("jq") !== null && Bun.which("perl") !== null;

type HookSpec = Record<string, { matcher?: string; command: string; timeout?: number }[]>;

/** Claude Code's settings shape: each command in a group of its own. */
function hooksConfig(spec: HookSpec): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(spec).map(([event, entries]) => [
      event,
      entries.map(({ matcher, command, timeout }) => ({
        ...(matcher && { matcher }),
        hooks: [{ type: "command", command, ...(timeout && { timeout }) }],
      })),
    ]),
  );
}

interface Setup {
  session: AgentSession;
  cwd: string;
  asked: PermissionAsk[];
}

async function setup(
  spec: HookSpec,
  model: FakeModel,
  options: {
    ask?: RequestPermission;
    config?: Partial<HarnessConfig>;
    subagent?: boolean;
    cwd?: string;
  } = {},
): Promise<Setup> {
  const cwd = options.cwd ?? (await tempDir());
  const asked: PermissionAsk[] = [];
  const agent = createNeoAgent({
    config: { ...TEST_CONFIG, hooks: hooksConfig(spec), ...options.config },
    streamFn: model.streamFn,
  });
  const ask: RequestPermission =
    options.ask ??
    ((request) => {
      asked.push(request);
      return Promise.resolve({ allowed: true });
    });
  const session = await agent.newSession({
    sessionId: "s1",
    cwd,
    mcpServers: [],
    requestPermission: ask,
    ...(options.subagent && { subagent: {} }),
  });
  return { session, cwd, asked };
}

async function run(
  session: AgentSession,
  text: string,
): Promise<{ updates: AgentUpdate[]; result: PromptResult }> {
  const updates: AgentUpdate[] = [];
  const result = await session.prompt({
    text,
    app: "cli",
    pendingEvents: [],
    onUpdate: (update) => updates.push(update),
  });
  return { updates, result };
}

const bashCall = (command: string, id = "b1"): ReturnType<typeof reply> =>
  reply([fauxToolCall("bash", { command, description: "test" }, { id })], "toolUse");

const outputOf = (updates: AgentUpdate[]): string =>
  ofType(updates, "tool_call_update")
    .flatMap((update) => update.content ?? [])
    .filter((content) => content.type === "text")
    .map((content) => content.text)
    .join("\n");

const notices = (updates: AgentUpdate[]): string[] =>
  ofType(updates, "notice").map((notice) => notice.text);

test("PreToolUse exit 2 blocks the call, and the agent reads stderr", async () => {
  const model = fakeModel([bashCall("touch made"), reply("ok")]);
  const { session, cwd, asked } = await setup(
    { PreToolUse: [{ matcher: "Bash", command: "echo 'no touching' >&2; exit 2" }] },
    model,
  );
  const { updates, result } = await run(session, "go");
  expect(result.stopReason).toBe("end_turn");
  expect(existsSync(join(cwd, "made"))).toBe(false);
  const last = ofType(updates, "tool_call_update").at(-1);
  expect(last?.status).toBe("failed");
  expect(outputOf(updates)).toContain("Blocked by a hook: no touching");
  expect(JSON.stringify(model.requests[1])).toContain("no touching");
  // The question was never asked.
  expect(asked).toEqual([]);
});

test("PreToolUse JSON deny blocks with its reason", async () => {
  const model = fakeModel([bashCall("touch made"), reply("ok")]);
  const json = JSON.stringify({
    hookSpecificOutput: { permissionDecision: "deny", permissionDecisionReason: "policy says no" },
  });
  const { session, cwd } = await setup(
    { PreToolUse: [{ command: `cat >/dev/null; echo '${json}'` }] },
    model,
  );
  const { updates } = await run(session, "go");
  expect(existsSync(join(cwd, "made"))).toBe(false);
  expect(outputOf(updates)).toContain("policy says no");
});

test("a matcher names Claude Code's tools: Bash matches bash, not read", async () => {
  const model = fakeModel([
    reply([fauxToolCall("read", { path: "f.txt" }, { id: "r1" })], "toolUse"),
    bashCall("echo hi"),
    reply("ok"),
  ]);
  const { session, cwd } = await setup(
    { PreToolUse: [{ matcher: "Bash", command: `cat >> hook.log; echo >> hook.log` }] },
    model,
  );
  await writeFile(join(cwd, "f.txt"), "x\n");
  await run(session, "go");
  const log = (await readFile(join(cwd, "hook.log"), "utf8")).trim().split("\n");
  expect(log).toHaveLength(1);
  const input = JSON.parse(log[0] ?? "{}");
  expect(input).toMatchObject({
    hook_event_name: "PreToolUse",
    session_id: "s1",
    cwd,
    tool_name: "Bash",
    tool_input: { command: "echo hi", description: "test" },
    tool_use_id: "b1",
  });
});

test("tool_input has Claude Code's argument names for write and edit", async () => {
  const model = fakeModel([
    reply([fauxToolCall("write", { path: "new.txt", content: "hello" }, { id: "w1" })], "toolUse"),
    reply("ok"),
  ]);
  const { session, cwd } = await setup(
    { PreToolUse: [{ matcher: "Edit|Write", command: "cat > seen.json" }] },
    model,
  );
  await run(session, "go");
  expect(JSON.parse(await readFile(join(cwd, "seen.json"), "utf8"))).toMatchObject({
    tool_name: "Write",
    tool_input: { file_path: "new.txt", content: "hello" },
  });
  expect(await readFile(join(cwd, "new.txt"), "utf8")).toBe("hello");
});

test("allow and ask go with the permission question; no decision leaves it alone", async () => {
  for (const [decision, expected] of [
    ["allow", "allow"],
    ["ask", "ask"],
    ["", undefined],
  ] as const) {
    const model = fakeModel([bashCall("true"), reply("ok")]);
    const out = JSON.stringify({ hookSpecificOutput: { permissionDecision: decision } });
    const { session, asked } = await setup(
      { PreToolUse: [{ command: `cat >/dev/null; echo '${out}'` }] },
      model,
    );
    await run(session, "go");
    expect(asked).toHaveLength(1);
    expect(asked[0]?.hook).toBe(expected);
  }
});

test("a hook's reason for allowing is shown to the user", async () => {
  const model = fakeModel([bashCall("true"), reply("ok")]);
  const json = JSON.stringify({
    hookSpecificOutput: { permissionDecision: "allow", permissionDecisionReason: "safe command" },
  });
  const { session } = await setup(
    { PreToolUse: [{ command: `cat >/dev/null; echo '${json}'` }] },
    model,
  );
  const { updates } = await run(session, "go");
  expect(notices(updates)).toEqual(["A hook allowed bash: safe command"]);
});

test("updatedInput rewrites the call's arguments", async () => {
  const model = fakeModel([bashCall("echo original"), reply("ok")]);
  const json = JSON.stringify({
    hookSpecificOutput: { updatedInput: { command: "echo rewritten" } },
  });
  const { session } = await setup(
    { PreToolUse: [{ command: `cat >/dev/null; echo '${json}'` }] },
    model,
  );
  const { updates } = await run(session, "go");
  expect(outputOf(updates)).toContain("rewritten");
  expect(outputOf(updates)).not.toContain("original");
});

test("a hook that fails without blocking shows in the chat and the call goes on", async () => {
  const model = fakeModel([bashCall("echo fine"), reply("ok")]);
  const { session } = await setup(
    { PreToolUse: [{ command: "echo 'script broke' >&2; exit 1" }] },
    model,
  );
  const { updates, result } = await run(session, "go");
  expect(result.stopReason).toBe("end_turn");
  expect(outputOf(updates)).toContain("fine");
  expect(ofType(updates, "notice")).toMatchObject([
    { tone: "danger", text: expect.stringContaining("script broke") },
  ]);
});

test("continue false stops the turn, and says why", async () => {
  const model = fakeModel([bashCall("touch made"), reply("never")]);
  const json = JSON.stringify({ continue: false, stopReason: "budget used up" });
  const { session, cwd } = await setup(
    { PreToolUse: [{ command: `cat >/dev/null; echo '${json}'` }] },
    model,
  );
  const { updates, result } = await run(session, "go");
  expect(result.stopReason).toBe("cancelled");
  expect(existsSync(join(cwd, "made"))).toBe(false);
  expect(model.requests).toHaveLength(1);
  expect(notices(updates).join("\n")).toContain("budget used up");
});

test("PostToolUse sees the response; exit 2 and context go to the agent with the result", async () => {
  const model = fakeModel([bashCall("echo out"), reply("ok")]);
  const { session, cwd } = await setup(
    {
      PostToolUse: [
        { matcher: "Bash", command: "cat > post.json; echo 'lint failed' >&2; exit 2" },
        {
          matcher: "Bash",
          command: `cat >/dev/null; echo '{"hookSpecificOutput":{"additionalContext":"remember X"}}'`,
        },
      ],
    },
    model,
  );
  const { updates } = await run(session, "go");
  const seen = JSON.parse(await readFile(join(cwd, "post.json"), "utf8"));
  expect(seen).toMatchObject({
    hook_event_name: "PostToolUse",
    tool_name: "Bash",
    tool_response: { stdout: "out" },
  });
  const result = JSON.stringify(model.requests[1]);
  expect(result).toContain("Hook feedback: lint failed");
  expect(result).toContain("Hook context: remember X");
  expect(outputOf(updates)).toContain("lint failed");
});

test("a failed tool call runs PostToolUseFailure, not PostToolUse", async () => {
  const model = fakeModel([bashCall("exit 3"), reply("ok")]);
  const { session, cwd } = await setup(
    {
      PostToolUse: [{ command: "touch wrong" }],
      PostToolUseFailure: [{ command: "cat > failure.json" }],
    },
    model,
  );
  await run(session, "go");
  expect(existsSync(join(cwd, "wrong"))).toBe(false);
  expect(JSON.parse(await readFile(join(cwd, "failure.json"), "utf8"))).toMatchObject({
    hook_event_name: "PostToolUseFailure",
    tool_name: "Bash",
    error: expect.stringContaining("exited with code 3"),
  });
});

test("UserPromptSubmit adds context to the prompt, and drops a blocked one", async () => {
  const model = fakeModel([reply("hello")]);
  const { session } = await setup(
    { UserPromptSubmit: [{ command: "cat >/dev/null; echo 'the date is 1999'" }] },
    model,
  );
  await run(session, "hi");
  const sent = JSON.stringify(model.requests[0]);
  expect(sent).toContain("the date is 1999");
  expect(sent).toContain("hi");

  const blockedModel = fakeModel([]);
  const blocked = await setup(
    { UserPromptSubmit: [{ command: "echo 'secrets in prompt' >&2; exit 2" }] },
    blockedModel,
  );
  const { updates, result } = await run(blocked.session, "my password is hunter2");
  expect(result.stopReason).toBe("end_turn");
  expect(blockedModel.requests).toHaveLength(0);
  expect(ofType(updates, "notice")).toMatchObject([
    { tone: "danger", text: "A hook dropped this prompt: secrets in prompt" },
  ]);
  // Nothing of it reached the conversation.
  expect(ofType(updates, "harness_message")).toEqual([]);
});

test("UserPromptSubmit gets the prompt as typed", async () => {
  const model = fakeModel([reply("hello")]);
  const { session, cwd } = await setup(
    { UserPromptSubmit: [{ command: "cat > prompt.json" }] },
    model,
  );
  await run(session, "say hi");
  expect(JSON.parse(await readFile(join(cwd, "prompt.json"), "utf8"))).toMatchObject({
    hook_event_name: "UserPromptSubmit",
    prompt: "say hi",
  });
});

test("SessionStart runs once, before the first prompt, and its context joins that prompt", async () => {
  const model = fakeModel([reply("one"), reply("two")]);
  const { session, cwd } = await setup(
    { SessionStart: [{ command: "cat >> starts.log; echo; echo 'project notes'" }] },
    model,
  );
  await run(session, "first");
  await run(session, "second");
  expect(JSON.stringify(model.requests[0])).toContain("project notes");
  expect(JSON.stringify(model.requests[1]).match(/project notes/g)).toHaveLength(1);
  const starts = (await readFile(join(cwd, "starts.log"), "utf8")).trim().split("\n");
  expect(starts).toHaveLength(1);
  expect(JSON.parse(starts[0] ?? "{}")).toMatchObject({ source: "startup" });
});

test("SessionEnd runs when the session is closed", async () => {
  const model = fakeModel([reply("one")]);
  const { session, cwd } = await setup({ SessionEnd: [{ command: "cat > end.json" }] }, model);
  await run(session, "first");
  await session.close?.("other");
  expect(JSON.parse(await readFile(join(cwd, "end.json"), "utf8"))).toMatchObject({
    hook_event_name: "SessionEnd",
    reason: "other",
  });
});

test("Stop can send the agent back to work, once told it's the second time", async () => {
  const model = fakeModel([reply("first answer"), reply("second answer")]);
  // Blocks until it is told stop_hook_active.
  const command = `input=$(cat); echo "$input" >> stops.log
if echo "$input" | grep -q '"stop_hook_active":true'; then exit 0; fi
echo "run the tests first" >&2; exit 2`;
  const { session, cwd } = await setup({ Stop: [{ command }] }, model);
  const { updates, result } = await run(session, "go");
  expect(result.stopReason).toBe("end_turn");
  expect(model.requests).toHaveLength(2);
  expect(JSON.stringify(model.requests[1])).toContain("run the tests first");
  expect(notices(updates).join("\n")).toContain("A Stop hook sent the agent back to work");
  const stops = (await readFile(join(cwd, "stops.log"), "utf8"))
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
  expect(stops.map((stop) => stop.stop_hook_active)).toEqual([false, true]);
  expect(stops[0]).toMatchObject({
    hook_event_name: "Stop",
    last_assistant_message: "first answer",
  });
});

test("a Stop hook that never relents is cut off", async () => {
  const model = fakeModel(Array.from({ length: 30 }, (_, i) => reply(`answer ${i}`)));
  const { session } = await setup({ Stop: [{ command: "echo again >&2; exit 2" }] }, model);
  const { updates, result } = await run(session, "go");
  expect(result.stopReason).toBe("end_turn");
  expect(model.requests).toHaveLength(21);
  expect(notices(updates).at(-1)).toContain("20 times");
});

test("a Stop hook with continue false ends the turn without sending the agent back", async () => {
  const model = fakeModel([reply("done")]);
  const json = JSON.stringify({ continue: false, stopReason: "all good" });
  const { session } = await setup({ Stop: [{ command: `cat >/dev/null; echo '${json}'` }] }, model);
  const { updates } = await run(session, "go");
  expect(model.requests).toHaveLength(1);
  expect(notices(updates).join("\n")).toContain("all good");
});

test("in a subagent, SubagentStop runs and Stop doesn't", async () => {
  const model = fakeModel([reply("done")]);
  const { session, cwd } = await setup(
    {
      Stop: [{ command: "touch stop-ran" }],
      SubagentStop: [{ command: "cat > substop.json" }],
    },
    model,
    { subagent: true },
  );
  await run(session, "go");
  expect(existsSync(join(cwd, "stop-ran"))).toBe(false);
  expect(JSON.parse(await readFile(join(cwd, "substop.json"), "utf8"))).toMatchObject({
    hook_event_name: "SubagentStop",
  });
});

test.skipIf(!HAVE_RALPH_TOOLS)("the transcript the hooks get is Claude Code's JSONL", async () => {
  const model = fakeModel([reply("the answer")]);
  const { session, cwd } = await setup(
    {
      Stop: [
        { command: `cat > stop.json; cp "$(jq -r .transcript_path stop.json)" transcript.jsonl` },
      ],
    },
    model,
  );
  await run(session, "question");
  const lines = (await readFile(join(cwd, "transcript.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
  expect(lines.map((line) => line.message.role)).toEqual(["user", "assistant"]);
  expect(lines[1]).toMatchObject({
    type: "assistant",
    message: { content: [{ type: "text", text: "the answer" }] },
  });
});

test.skipIf(!HAVE_RALPH_TOOLS)(
  "ralph-loop's real Stop hook keeps the loop going from the transcript",
  async () => {
    const plugins = await tempDir();
    const root = join(plugins, "ralph-loop");
    await cp(join(import.meta.dir, "fixtures", "plugins", "ralph-loop"), root, { recursive: true });
    await writeFile(
      join(plugins, "installed.json"),
      JSON.stringify({ version: 1, plugins: { "ralph-loop@official": { root, version: "1" } } }),
    );
    const cwd = await tempDir();
    await mkdir(join(cwd, ".claude"), { recursive: true });
    await writeFile(
      join(cwd, ".claude", "ralph-loop.local.md"),
      `---\nactive: true\niteration: 1\nmax_iterations: 3\ncompletion_promise: "DONE"\n---\nBuild the thing`,
    );
    const model = fakeModel([
      reply("working"),
      reply("still working"),
      reply("<promise>DONE</promise>"),
    ]);
    const { session } = await setup({}, model, { config: { plugins, hooks: undefined }, cwd });
    const { updates, result } = await run(session, "start");
    expect(result.stopReason).toBe("end_turn");
    // Twice back to work with the saved prompt, then the promise ends it.
    expect(model.requests).toHaveLength(3);
    expect(JSON.stringify(model.requests[1])).toContain("Build the thing");
    expect(notices(updates).some((text) => text.includes("Stop hook sent the agent back"))).toBe(
      true,
    );
    expect(existsSync(join(cwd, ".claude", "ralph-loop.local.md"))).toBe(false);
  },
);

test.skipIf(!HAVE_PYTHON)(
  "hookify's real scripts block a command by Claude Code's names",
  async () => {
    const plugins = await tempDir();
    const root = join(plugins, "hookify");
    await cp(join(import.meta.dir, "fixtures", "plugins", "hookify"), root, { recursive: true });
    await writeFile(
      join(plugins, "installed.json"),
      JSON.stringify({ version: 1, plugins: { "hookify@official": { root, version: "1" } } }),
    );
    const cwd = await tempDir();
    await mkdir(join(cwd, ".claude"), { recursive: true });
    await writeFile(
      join(cwd, ".claude", "hookify.no-rm.local.md"),
      "---\nname: block-dangerous-rm\nenabled: true\nevent: bash\npattern: rm\\s+-rf\naction: block\n---\nDangerous rm!\n",
    );
    const model = fakeModel([
      bashCall("rm -rf build", "b1"),
      bashCall("echo fine", "b2"),
      reply("ok"),
    ]);
    const { session } = await setup({}, model, { config: { plugins, hooks: undefined }, cwd });
    const { updates, result } = await run(session, "go");
    expect(result.stopReason).toBe("end_turn");
    const calls = ofType(updates, "tool_call_update").filter(
      (u) => u.status === "failed" || u.status === "completed",
    );
    expect(calls.map((call) => call.status)).toEqual(["failed", "completed"]);
    expect(outputOf(updates)).toContain("Dangerous rm!");
    expect(outputOf(updates)).toContain("fine");
    // Its message is for the user too.
    expect(notices(updates).join("\n")).toContain("block-dangerous-rm");
  },
);

test("project hooks are read from .suite/settings.json", async () => {
  const cwd = await tempDir();
  await mkdir(join(cwd, ".suite"), { recursive: true });
  await writeFile(
    join(cwd, ".suite", "settings.json"),
    JSON.stringify({
      hooks: {
        PreToolUse: [
          { matcher: "Bash", hooks: [{ type: "command", command: "echo no >&2; exit 2" }] },
        ],
      },
    }),
  );
  const model = fakeModel([bashCall("touch made"), reply("ok")]);
  const { session } = await setup({}, model, { cwd, config: { hooks: undefined } });
  await run(session, "go");
  expect(existsSync(join(cwd, "made"))).toBe(false);
});

test("without hooks nothing changes", async () => {
  const model = fakeModel([bashCall("echo plain"), reply("ok")]);
  const { session, asked } = await setup({}, model, { config: { hooks: undefined } });
  const { updates } = await run(session, "go");
  expect(outputOf(updates)).toContain("plain");
  expect(asked[0]?.hook).toBeUndefined();
  expect(ofType(updates, "notice")).toEqual([]);
});

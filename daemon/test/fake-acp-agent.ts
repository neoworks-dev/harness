// A scripted ACP agent (stdio, one JSON-RPC message per line) standing in for Claude Code or
// Codex in guest tests. What it does depends on the task, the prompt's last line:
//   greet <name>  calls greeter.greet through neo's MCP server (the gateway)
//   ask           asks for permission and reports the answer
//   ask edit      the same, for a tool call of kind edit with a diff (ask execute: kind execute;
//                 ask diff: a diff but no kind; ask bash: a command with its description, as
//                 Claude Code asks; ask agent: Claude Code's Agent tool, with an always option)
//   wait          runs until cancelled
//   edit          reports a tool call with a diff
//   readfile <p>  asks to read the file (kind read, locations [p]) and reports the answer
//   runcmd <c>    asks to run the command (kind execute, rawInput.command) and reports the answer
//   editfile <p> <text>  asks to edit the file, and when allowed writes it and reports the
//                 completed tool call with its diff (like an adapter that asks before it writes)
//   config        reports what session/new (or resume) and set_config_option gave it (the
//                 instructions, the disallowed tools, the ask rules, the settings)
//   prompts       reports what the system prompt and the tools came to (see below)
//   neocall <tool> <json>  calls one of neo's tools through the gateway and replies its result
//   title <t>     replies {"title": "<t>"} (plus " (<model>)" once a model was set): a title
//                 generateText asked for (the prompt's last line)
//   started       reports how the conversation began: "new" or "resume" (resuming a session id
//                 that starts with "gone" fails, like an adapter that lost the conversation)
//   neotool <t> asks for permission to use neo's own MCP tool <t> (titled
//                 mcp__neo__<t>; "codex <t>": mcp.neo.<t>) and reports the answer
//   cwd           reports the directory session/new (or resume) was given
//   route         reports how the process was started: the CODEX_CONFIG, ANTHROPIC_*,
//                 CLAUDE_CODE_{OAUTH,USE_,EXECUTABLE} and MODEL_PROVIDER variables it has, the initialize params
//                 and the params of an authenticate request (Codex's gateway sign-in; header values are the
//                 first 12 hex digits of their sha256, so the report holds no key)
//   usage         sends usage_updates in the middle of one message (USD cost, then EUR cost)
//   send <to> <text>  calls neo's send_message and reports "sent: …" (or the error)
//   spam <to>     sends 25 messages to <to> and reports how many were taken and the last answer
//   listen        calls neo's wait (5 s) and reports "heard: …"
//   poll          calls tools_search until a message rides on a result, and reports it
//   commands      replaces its slash commands with one (review) and says so
//   subagent      runs Claude Code's Agent tool as the ACP RFD has it: an Explore subagent in a
//                 session of its own (sub-1), announced and ended with subagent_update ("subagent
//                 ask": it asks for permission in its session first)
//   background subagent  an Agent call run in the background: the reply and its cost come at
//                 once, the prompt stays open until the subagent is done (some 400 ms)
//   spawn         Codex's spawn_agent as AIR has it: subagent_spawned, then it fails
// It lists slash commands (available_commands_update) when a conversation starts or resumes: after
// the answer to session/new, or before it with FAKE_ACP_COMMANDS_FIRST in the environment.
// It offers a model option (small, large) and an effort option (low, high), and one stored
// conversation, "old-1", for session/list and session/resume.
//   anything else sends a plan (one entry with an unknown status), a usage_update, and replies
//   "Done: <task>"
// FAKE_ACP_HANG in the environment makes session/new never answer. FAKE_ACP_STEERING makes it
// take _session/steering; `ask` then reports what it was steered with after the answer.
// Every prompt first echoes the whole prompt text as a thought, so tests can see the event tags.
import { readFileSync, writeFileSync } from "node:fs";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

interface HttpServer {
  url: string;
  headers: { name: string; value: string }[];
}

let nextId = 1000;
const waiting = new Map<number, (result: unknown) => void>();
let gateway: HttpServer | undefined;
let instructions: unknown;
/** How the instructions end, the system prompt in place of the adapter's own, Codex's. */
let instructionsEnd: unknown;
let systemPrompt: unknown;
let disallowed: unknown;
/** The ask and allow rules of the settings Claude Code's adapter was given. */
let ask: unknown;
let allow: unknown;
/** The gateway's tools (tools/list), as this adapter sees them. */
let gatewayTools: unknown;
const config: Record<string, unknown> = {};
let sessionId = "";
let started = "new";
let cwd = "";
let initialize: unknown;
let authenticated: unknown;
/** What `_session/steering` brought into the running turn (FAKE_ACP_STEERING). */
const steered: string[] = [];

const VALUES: Record<string, string[]> = { model: ["small", "large"], effort: ["low", "high"] };
const CATEGORY: Record<string, string> = { model: "model", effort: "thought_level" };

function configOptions(): unknown[] {
  return Object.entries(VALUES).map(([id, values]) => ({
    id,
    category: CATEGORY[id],
    type: "select",
    currentValue: (config[id] as string | undefined) ?? values[0],
    options: values.map((value) => ({
      value,
      name: value.toUpperCase(),
      description: `${value} one`,
    })),
  }));
}
let cancel: (() => void) | undefined;

function listCommands(sessionId: string, commands: Record<string, unknown>[]): void {
  update(sessionId, { sessionUpdate: "available_commands_update", availableCommands: commands });
}

const START_COMMANDS = [
  { name: "review", description: "Review the current changes", input: { hint: "branch or PR" } },
  { name: "/compact", description: "Summarise the conversation" },
  { name: "model", description: "Claude's own model picker" },
  { name: "", description: "nameless" },
];
// FAKE_ACP_MCP: /mcp as Claude Code's adapter answers it (its formatMcpStatus's markdown).
if (process.env.FAKE_ACP_MCP !== undefined)
  START_COMMANDS.push({ name: "mcp", description: "MCP" });
const mcpServers: Record<string, string> = {
  serena: "connected",
  posthog: "disabled",
  broken: "failed",
};

function mcpMarkdown(): string {
  const of = (status: string): string[] =>
    Object.keys(mcpServers).filter((name) => mcpServers[name] === status);
  const blocks = ["3 MCP servers"];
  if (of("failed").length > 0) {
    blocks.push(
      ["**Failed**", ...of("failed").map((n) => `- \`${n}\`: Connection closed`)].join("\n"),
    );
  }
  if (of("connected").length > 0) {
    blocks.push(["**Connected**", ...of("connected").map((n) => `- \`${n}\`: 4 tools`)].join("\n"));
  }
  if (of("disabled").length > 0) {
    blocks.push(
      `**Disabled:** ${of("disabled")
        .map((n) => `\`${n}\``)
        .join(", ")}`,
    );
  }
  return blocks.join("\n\n");
}

function send(message: Record<string, unknown>): void {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}

function request(method: string, params: unknown): Promise<unknown> {
  const id = nextId++;
  send({ id, method, params });
  return new Promise((resolve) => waiting.set(id, resolve));
}

function update(sessionId: string, value: Record<string, unknown>): void {
  send({ method: "session/update", params: { sessionId, update: value } });
}

function say(sessionId: string, text: string, kind = "agent_message_chunk"): void {
  update(sessionId, { sessionUpdate: kind, content: { type: "text", text } });
}

let neoClient: Promise<Client> | undefined;

/** A call of one of neo's core tools as this agent: the text of the result, and whether it failed. */
async function neo(
  name: string,
  args: Record<string, unknown>,
): Promise<{ text: string; isError: boolean }> {
  if (!gateway) throw new Error("no MCP server");
  const { url, headers } = gateway;
  neoClient ??= (async () => {
    const client = new Client({ name: "fake-guest", version: "1" });
    const requestInit = { headers: Object.fromEntries(headers.map((h) => [h.name, h.value])) };
    await client.connect(new StreamableHTTPClientTransport(new URL(url), { requestInit }));
    return client;
  })();
  const result = await (await neoClient).callTool({ name, arguments: args });
  const blocks = result.content as { text?: string }[];
  return { text: blocks.map((b) => b.text ?? "").join("\n"), isError: result.isError === true };
}

async function greet(sessionId: string, name: string): Promise<void> {
  if (!gateway) throw new Error("no MCP server");
  const headers = Object.fromEntries(gateway.headers.map((h) => [h.name, h.value]));
  const client = new Client({ name: "fake-guest", version: "1" });
  await client.connect(
    new StreamableHTTPClientTransport(new URL(gateway.url), { requestInit: { headers } }),
  );
  const rawInput = { name: "greeter.greet", args: { name } };
  update(sessionId, {
    sessionUpdate: "tool_call",
    toolCallId: "t1",
    title: "mcp__neo__tools_call",
    kind: "other",
    status: "pending",
    rawInput: {},
  });
  // Like Claude Code's adapter: the input arrives after the call started.
  update(sessionId, { sessionUpdate: "tool_call_update", toolCallId: "t1", rawInput });
  const result = await client.callTool({ name: "tools_call", arguments: rawInput });
  await client.close();
  const blocks = result.content as { text?: string }[];
  const text = blocks.map((b) => b.text ?? "").join("\n");
  update(sessionId, {
    sessionUpdate: "tool_call_update",
    toolCallId: "t1",
    status: "completed",
    content: [{ type: "content", content: { type: "text", text } }],
  });
}

const OPTIONS = [
  { optionId: "yes", name: "Allow", kind: "allow_once" },
  { optionId: "no", name: "Reject", kind: "reject_once" },
];

/** Asks for permission; resolves with the picked option id (or the outcome when cancelled). */
async function permission(sessionId: string, toolCall: Record<string, unknown>): Promise<string> {
  const answer = (await request("session/request_permission", {
    sessionId,
    toolCall,
    options: OPTIONS,
  })) as { outcome: { outcome: string; optionId?: string } };
  return answer.outcome.optionId ?? answer.outcome.outcome;
}

/** The authenticate params with header values as sha256 prefixes, so a report holds no key. */
function withHashedHeaders(params: Record<string, unknown>): unknown {
  interface Shape {
    _meta?: { gateway?: { headers?: Record<string, string> } };
  }
  const copy = structuredClone<Shape>(params);
  const headers = copy._meta?.gateway?.headers ?? {};
  for (const [name, value] of Object.entries(headers)) {
    headers[name] = new Bun.CryptoHasher("sha256").update(value).digest("hex").slice(0, 12);
  }
  return copy;
}

function readOrUndefined(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

async function prompt(params: {
  sessionId: string;
  prompt: { text?: string }[];
}): Promise<unknown> {
  const { sessionId } = params;
  const text = params.prompt.map((b) => b.text ?? "").join("\n");
  const task = text.split("\n").at(-1) ?? "";
  const mcp = /^\/mcp(?: (enable|disable|reconnect) (\S+))?$/.exec(text.trim());
  if (mcp && process.env.FAKE_ACP_MCP !== undefined) {
    const [, action, name] = mcp;
    if (action !== undefined && name !== undefined) {
      mcpServers[name] = action === "disable" ? "disabled" : "connected";
    }
    say(sessionId, mcpMarkdown());
    return { stopReason: "end_turn" };
  }
  say(sessionId, text, "agent_thought_chunk");
  say(sessionId, "Looking.");
  if (task === "ask then work") {
    // Asks, then works on a while: a session/cancel meanwhile ends the turn as cancelled.
    const answer = await permission(sessionId, { toolCallId: "t7", kind: "execute", title: "ls" });
    say(sessionId, `answer: ${answer}`);
    const stopped = await Promise.race([
      new Promise<boolean>((resolve) => (cancel = () => resolve(true))),
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 300)),
    ]);
    if (stopped) return { stopReason: "cancelled" };
    say(sessionId, "went on");
    return { stopReason: "end_turn" };
  }
  if (task.startsWith("greet ")) {
    await greet(sessionId, task.slice(6));
  } else if (task === "ask" || task.startsWith("ask ")) {
    const what = task.slice(4);
    const toolCall: Record<string, unknown> = { toolCallId: "t2", title: "Write notes.txt" };
    const diff = { type: "diff", path: "/tmp/notes.txt", oldText: "a\n", newText: "b\n" };
    if (what === "edit") Object.assign(toolCall, { kind: "edit", content: [diff] });
    else if (what === "diff") Object.assign(toolCall, { content: [diff] });
    else if (what === "execute") Object.assign(toolCall, { kind: "execute", title: "ls" });
    else if (what === "agent") {
      // Claude Code's Agent tool, asked about through an ask rule.
      Object.assign(toolCall, {
        kind: "think",
        title: "Agent",
        rawInput: {
          subagent_type: "Explore",
          description: "Find the tests",
          prompt: "List the test files.",
        },
      });
    } else if (what === "bash") {
      // As Claude Code asks: the command as the title, its description as the content.
      Object.assign(toolCall, {
        kind: "execute",
        title: "ls -la",
        rawInput: { command: "ls -la", description: "List the files" },
        content: [{ type: "content", content: { type: "text", text: "List the files" } }],
      });
    }
    const options = [{ optionId: "yes", name: "Allow", kind: "allow_once" }];
    if (what === "agent")
      options.push({ optionId: "always", name: "Always", kind: "allow_always" });
    options.push({ optionId: "no", name: "Reject", kind: "reject_once" });
    const answer = (await request("session/request_permission", {
      sessionId,
      toolCall,
      options,
    })) as { outcome: { outcome: string; optionId?: string } };
    say(sessionId, "\n\n");
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${answer.outcome.optionId ?? answer.outcome.outcome}`);
    if (process.env.FAKE_ACP_STEERING !== undefined) {
      // What the client steers in right after the answer (a note on it) is part of this turn.
      await new Promise((resolve) => setTimeout(resolve, 100));
      for (const text of steered.splice(0)) say(sessionId, `\nsteered: ${text}`);
    }
    return { stopReason: "end_turn" };
  } else if (task === "later") {
    // Like Claude Code once a background command ended: a cycle of its own, after the turn,
    // that only its result's usage ends.
    setTimeout(() => {
      say(sessionId, "The build passed.");
      update(sessionId, { sessionUpdate: "usage_update", used: 10, size: 100 });
      update(sessionId, {
        sessionUpdate: "usage_update",
        used: 12,
        size: 100,
        cost: { amount: 0.01, currency: "USD" },
      });
    }, 50);
    say(sessionId, "Started the build.");
    return { stopReason: "end_turn" };
  } else if (task === "commands") {
    listCommands(sessionId, [{ name: "review", description: "Review again" }]);
    say(sessionId, "listed");
    return { stopReason: "end_turn" };
  } else if (task.startsWith("send ")) {
    const [to = "", ...rest] = task.slice(5).split(" ");
    const sent = await neo("send_message", { to, text: rest.join(" ") });
    say(sessionId, `${sent.isError ? "refused" : "sent"}: ${sent.text}`);
    return { stopReason: "end_turn" };
  } else if (task.startsWith("spam ")) {
    let taken = 0;
    let last = "";
    for (let i = 0; i < 25; i++) {
      const sent = await neo("send_message", { to: task.slice(5), text: `ping ${i}` });
      if (!sent.isError) taken += 1;
      last = sent.text;
    }
    say(sessionId, `taken: ${taken}; last: ${last}`);
    return { stopReason: "end_turn" };
  } else if (task === "listen") {
    const heard = await neo("wait", { timeoutSeconds: 5 });
    say(sessionId, `heard: ${heard.text}`);
    return { stopReason: "end_turn" };
  } else if (task === "poll") {
    let text = "";
    for (let i = 0; i < 200 && !text.includes("<message"); i++) {
      text = (await neo("tools_search", { query: "zzz" })).text;
      await Bun.sleep(30);
    }
    say(sessionId, `polled: ${text}`);
    return { stopReason: "end_turn" };
  } else if (task.startsWith("readfile ")) {
    const path = task.slice(9);
    const toolCall = {
      toolCallId: "t5",
      title: `Read ${path}`,
      kind: "read",
      locations: [{ path }],
    };
    const picked = await permission(sessionId, toolCall);
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${picked}`);
    return { stopReason: "end_turn" };
  } else if (task.startsWith("neotool ") || task.startsWith("codex ")) {
    const [style, name = ""] = task.split(" ");
    const title = `${style === "codex" ? "mcp.neo." : "mcp__neo__"}${name}`;
    const picked = await permission(sessionId, { toolCallId: "t9", title, kind: "other" });
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${picked}`);
    return { stopReason: "end_turn" };
  } else if (task === "route") {
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([name]) =>
        /^(CODEX_CONFIG|MODEL_PROVIDER|ANTHROPIC_|CLAUDE_CODE_(OAUTH|USE_|EXECUTABLE))/.test(name),
      ),
    );
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, JSON.stringify({ env, initialize, authenticated }));
    return { stopReason: "end_turn" };
  } else if (task === "cwd") {
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `cwd: ${cwd}`);
    return { stopReason: "end_turn" };
  } else if (task.startsWith("runcmd ")) {
    const command = task.slice(7);
    const toolCall = {
      toolCallId: "t6",
      title: "Bash",
      kind: "execute",
      rawInput: { command },
    };
    const picked = await permission(sessionId, toolCall);
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${picked}`);
    return { stopReason: "end_turn" };
  } else if (task.startsWith("editfile ")) {
    const [path = "", ...rest] = task.slice(9).split(" ");
    const newText = rest.join(" ");
    const oldText = readOrUndefined(path);
    const diff = { type: "diff", path, oldText, newText };
    const toolCall = { toolCallId: "t7", title: `Edit ${path}`, kind: "edit", content: [diff] };
    const picked = await permission(sessionId, toolCall);
    if (picked === "yes") {
      writeFileSync(path, newText);
      update(sessionId, {
        sessionUpdate: "tool_call",
        ...toolCall,
        status: "pending",
        content: [],
      });
      update(sessionId, {
        sessionUpdate: "tool_call_update",
        toolCallId: "t7",
        status: "completed",
        content: [diff],
      });
    }
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${picked}`);
    return { stopReason: "end_turn" };
  } else if (task === "wait") {
    await new Promise<void>((resolve) => (cancel = resolve));
    return { stopReason: "cancelled" };
  } else if (task === "edit") {
    update(sessionId, {
      sessionUpdate: "tool_call",
      toolCallId: "t3",
      title: "Edit a.txt",
      kind: "edit",
      status: "completed",
      content: [{ type: "diff", path: "/tmp/a.txt", oldText: "a\n", newText: "b\n" }],
    });
  } else if (task === "image") {
    // A tool that returns a picture (a 1x1 png) as an image block.
    const data =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    update(sessionId, {
      sessionUpdate: "tool_call",
      toolCallId: "t4",
      title: "Read dot.png",
      kind: "read",
      status: "completed",
      content: [{ type: "content", content: { type: "image", data, mimeType: "image/png" } }],
    });
  } else if (task === "config") {
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, JSON.stringify({ instructions, disallowed, ask, config, session: sessionId }));
    return { stopReason: "end_turn" };
  } else if (task === "prompts") {
    // What the system prompt and the tools came to: how the instructions end, the prompt in
    // place of the adapter's own (Claude Code's; Codex's base_instructions), the allow rules,
    // and the gateway's tools.
    let base: unknown;
    try {
      const codex: { base_instructions?: unknown } = JSON.parse(process.env.CODEX_CONFIG ?? "{}");
      base = codex.base_instructions;
    } catch {
      base = undefined;
    }
    if (gateway && gatewayTools === undefined) {
      const requestInit = {
        headers: Object.fromEntries(gateway.headers.map((h) => [h.name, h.value])),
      };
      try {
        const client = new Client({ name: "fake-guest", version: "1" });
        await client.connect(
          new StreamableHTTPClientTransport(new URL(gateway.url), { requestInit }),
        );
        gatewayTools = (await client.listTools()).tools.map((t) => t.name);
        await client.close();
      } catch (err) {
        gatewayTools = `failed: ${String(err)}`;
      }
    }
    const start = (v: unknown): unknown => (typeof v === "string" ? v.slice(0, 30) : v);
    say(
      sessionId,
      JSON.stringify({
        instructionsEnd,
        systemPrompt: start(systemPrompt),
        systemPromptEnd: typeof systemPrompt === "string" ? systemPrompt.slice(-20) : undefined,
        base: start(base),
        disallowed,
        allow,
        tools: gatewayTools,
      }),
    );
    return { stopReason: "end_turn" };
  } else if (task.startsWith("neocall ")) {
    // neocall <tool> <json args>: one of neo's tools through the gateway, its result as the reply.
    const [, name = "", ...rest] = task.split(" ");
    const args: Record<string, unknown> = JSON.parse(rest.join(" ") || "{}");
    const result = await neo(name, args);
    say(sessionId, JSON.stringify(result));
    return { stopReason: "end_turn" };
  } else if (task.startsWith("title ")) {
    // What generateText asks: the reply is a title (with the model it was asked on, if one was set).
    let title = task.slice(6);
    if (typeof config.model === "string") title += ` (${config.model})`;
    say(sessionId, JSON.stringify({ title }));
    return { stopReason: "end_turn" };
  } else if (task === "started") {
    say(sessionId, `started: ${started}`);
    return { stopReason: "end_turn" };
  } else if (task === "subagent" || task === "subagent ask") {
    // Claude Code's Agent tool in the ACP RFD's form: the call stays the parent's, the subagent
    // reports in a session of its own, announced and ended on the parent.
    const input = { subagent_type: "Explore", description: "Find the tests", prompt: "List them." };
    const call = { toolCallId: "t-agent", title: "Agent", kind: "think", rawInput: input };
    update(sessionId, { sessionUpdate: "tool_call", ...call, status: "pending" });
    const child = "sub-1";
    update(sessionId, {
      sessionUpdate: "subagent_update",
      sessionId: child,
      title: "Explore",
      description: "Find the tests",
      capabilities: { cancel: {} },
      state: { state: "running" },
    });
    update(child, {
      sessionUpdate: "session_message",
      messageId: "t-agent",
      senderSessionId: sessionId,
      recipientSessionId: child,
      content: [{ type: "text", text: "List them." }],
    });
    update(child, {
      sessionUpdate: "tool_call",
      toolCallId: "t-glob",
      title: "Glob test/**",
      kind: "search",
      status: "completed",
    });
    if (task === "subagent ask") {
      say(child, `asked: ${await permission(child, { toolCallId: "t-rm", title: "rm x" })}`);
    }
    say(child, "Found 3 tests.");
    update(sessionId, {
      sessionUpdate: "subagent_update",
      sessionId: child,
      state: { state: "idle", stopReason: "end_turn" },
    });
    update(sessionId, {
      sessionUpdate: "tool_call_update",
      toolCallId: "t-agent",
      status: "completed",
    });
    say(sessionId, "It found them.");
    return { stopReason: "end_turn" };
  } else if (task === "background subagent") {
    // Claude Code with an Agent call run in the background: its reply and result (the usage with
    // the cost) come at once, but the prompt stays open until the subagent is done.
    const input = {
      subagent_type: "general-purpose",
      description: "Research the cache",
      prompt: "Find out.",
      run_in_background: true,
    };
    const call = { toolCallId: "t-bg", title: "Agent", kind: "think", rawInput: input };
    update(sessionId, { sessionUpdate: "tool_call", ...call, status: "pending" });
    const child = "sub-bg";
    update(sessionId, {
      sessionUpdate: "subagent_update",
      sessionId: child,
      title: "general-purpose",
      description: "Research the cache",
      state: { state: "running" },
    });
    update(child, {
      sessionUpdate: "session_message",
      messageId: "t-bg",
      content: [{ type: "text", text: "Find out." }],
    });
    update(sessionId, {
      sessionUpdate: "tool_call_update",
      toolCallId: "t-bg",
      status: "completed",
    });
    say(sessionId, "It runs in the background.");
    update(sessionId, {
      sessionUpdate: "usage_update",
      used: 100,
      size: 2000,
      cost: { amount: 0.01, currency: "USD" },
    });
    await Bun.sleep(400);
    say(child, "Five minutes.");
    update(sessionId, {
      sessionUpdate: "subagent_update",
      sessionId: child,
      state: { state: "idle", stopReason: "end_turn" },
    });
    say(sessionId, "It says five minutes.");
    update(sessionId, {
      sessionUpdate: "usage_update",
      used: 120,
      size: 2000,
      cost: { amount: 0.02, currency: "USD" },
    });
    return { stopReason: "end_turn" };
  } else if (task === "spawn") {
    // Codex's spawn_agent in AIR's form.
    update(sessionId, {
      sessionUpdate: "subagent_spawned",
      subagentSessionId: "thread-2",
      name: "worker",
      task: "Delegated task for worker",
      prompt: "Count the lines.",
      capabilities: {},
    });
    say("thread-2", "42 lines.");
    update(sessionId, {
      sessionUpdate: "subagent_state_update",
      subagentSessionId: "thread-2",
      state: "failed",
    });
    say(sessionId, "The worker is done.");
    return { stopReason: "end_turn" };
  } else if (task === "usage") {
    say(sessionId, "one ");
    update(sessionId, { sessionUpdate: "usage_update", used: 100, size: 2000 });
    say(sessionId, "two ");
    update(sessionId, {
      sessionUpdate: "usage_update",
      used: 300,
      size: 2000,
      cost: { amount: 1.5, currency: "EUR" },
    });
    say(sessionId, "three");
    return { stopReason: "end_turn" };
  }
  update(sessionId, {
    sessionUpdate: "plan",
    entries: [
      { content: "Read the code", priority: "high", status: "completed" },
      { content: "Make the change", priority: "medium", status: "in_progress" },
      { content: "Check it", priority: "urgent", status: "later" },
    ],
  });
  update(sessionId, {
    sessionUpdate: "usage_update",
    used: 21109,
    size: 1000000,
    cost: { amount: 0.25, currency: "USD" },
  });
  say(sessionId, `Done: ${task}`);
  return { stopReason: "end_turn" };
}

async function handle(message: Record<string, unknown>): Promise<void> {
  const { id, method } = message;
  const params = message.params as Record<string, unknown>;
  if (method === undefined) {
    waiting.get(id as number)?.(message.result);
    return;
  }
  try {
    let result: unknown = null;
    if (method === "initialize") {
      initialize = params;
      result = {
        protocolVersion: 1,
        agentCapabilities: { mcpCapabilities: { http: true } },
        // As claude-agent-acp with _session/background_tasks (and steering) would.
        _meta: {
          backgroundTasks: { supported: true },
          ...(process.env.FAKE_ACP_STEERING !== undefined && { steering: { supported: true } }),
        },
      };
    } else if (method === "authenticate") {
      authenticated = withHashedHeaders(params);
      result = {};
    } else if (method === "session/new") {
      // FAKE_ACP_HANG: an adapter that starts but never gets a conversation going.
      if (process.env.FAKE_ACP_HANG !== undefined) await new Promise(() => undefined);
      const servers = params.mcpServers as HttpServer[];
      gateway = servers[0];
      cwd = params.cwd as string;
      const meta = (params._meta ?? {}) as {
        jetbrains?: { air?: { customInstructions?: string } };
        systemPrompt?: string;
        claudeCode?: {
          options?: {
            disallowedTools?: string[];
            settings?: { permissions?: { ask?: string[]; allow?: string[] } };
          };
        };
      };
      instructions = meta.jetbrains?.air?.customInstructions?.slice(0, 40);
      instructionsEnd = meta.jetbrains?.air?.customInstructions?.slice(-20);
      systemPrompt = meta.systemPrompt;
      disallowed = meta.claudeCode?.options?.disallowedTools;
      ask = meta.claudeCode?.options?.settings?.permissions?.ask;
      allow = meta.claudeCode?.options?.settings?.permissions?.allow;
      sessionId = "acp-1";
      result = { sessionId, configOptions: configOptions() };
      if (process.env.FAKE_ACP_COMMANDS_FIRST !== undefined)
        listCommands(sessionId, START_COMMANDS);
    } else if (method === "session/resume") {
      if ((params.sessionId as string).startsWith("gone")) throw new Error("no such session");
      gateway = (params.mcpServers as HttpServer[])[0];
      cwd = params.cwd as string;
      sessionId = params.sessionId as string;
      started = "resume";
      result = { configOptions: configOptions() };
    } else if (method === "session/list") {
      result = {
        sessions: [
          {
            sessionId: "old-1",
            cwd: params.cwd,
            title: "Earlier work",
            updatedAt: "2026-10-01T09:30:00Z",
          },
        ],
      };
    } else if (method === "session/set_config_option") {
      const key = params.configId as string;
      if (!VALUES[key]?.includes(params.value as string)) {
        throw new Error(`no ${key} ${String(params.value)}`);
      }
      config[key] = params.value;
      result = { configOptions: configOptions() };
    } else if (method === "session/prompt") {
      result = await prompt(params as never);
    } else if (method === "session/cancel") {
      cancel?.();
      return;
    } else if (method === "_session/steering") {
      const prompt = params.prompt as { text?: string }[];
      steered.push(prompt.map((b) => b.text ?? "").join("\n"));
      result = { outcome: "injected" };
    } else if (method === "_session/background_tasks") {
      // Only the call the tests name, by the id the adapter knows it by.
      result = { backgrounded: params.toolCallId === "t-wait" };
    } else {
      send({ id, error: { code: -32601, message: `no ${JSON.stringify(method)}` } });
      return;
    }
    if (id !== undefined) send({ id, result });
    if (
      (method === "session/new" || method === "session/resume") &&
      process.env.FAKE_ACP_COMMANDS_FIRST === undefined
    ) {
      listCommands(sessionId, START_COMMANDS);
    }
  } catch (err) {
    send({ id, error: { code: -32603, message: String(err) } });
  }
}

let buffer = "";
for await (const chunk of Bun.stdin.stream()) {
  buffer += new TextDecoder().decode(chunk);
  let nl = buffer.indexOf("\n");
  while (nl !== -1) {
    const line = buffer.slice(0, nl);
    buffer = buffer.slice(nl + 1);
    if (line.trim() !== "") void handle(JSON.parse(line));
    nl = buffer.indexOf("\n");
  }
}

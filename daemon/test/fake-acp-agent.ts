// A scripted ACP agent (stdio, one JSON-RPC message per line) standing in for Claude Code or
// Codex in guest tests. What it does depends on the task, the prompt's last line:
//   greet <name>  calls greeter.greet through the suite's MCP server (the gateway)
//   ask           asks for permission and reports the answer
//   ask edit      the same, for a tool call of kind edit with a diff (ask execute: kind execute;
//                 ask diff: a diff but no kind)
//   wait          runs until cancelled
//   edit          reports a tool call with a diff
//   readfile <p>  asks to read the file (kind read, locations [p]) and reports the answer
//   runcmd <c>    asks to run the command (kind execute, rawInput.command) and reports the answer
//   editfile <p> <text>  asks to edit the file, and when allowed writes it and reports the
//                 completed tool call with its diff (like an adapter that asks before it writes)
//   config        reports what session/new (or resume) and set_config_option gave it
//   started       reports how the conversation began: "new" or "resume" (resuming a session id
//                 that starts with "gone" fails, like an adapter that lost the conversation)
//   suitetool <t> asks for permission to use the suite's own MCP tool <t> (titled
//                 mcp__suite__<t>; "codex <t>": mcp.suite.<t>) and reports the answer
//   cwd           reports the directory session/new (or resume) was given
//   route         reports how the process was started: the CODEX_CONFIG, ANTHROPIC_*,
//                 CLAUDE_CODE_{OAUTH,USE_,EXECUTABLE} and MODEL_PROVIDER variables it has, the initialize params
//                 and the params of an authenticate request (Codex's gateway sign-in; header values are the
//                 first 12 hex digits of their sha256, so the report holds no key)
//   usage         sends usage_updates in the middle of one message (USD cost, then EUR cost)
// It offers a model option (small, large) and an effort option (low, high), and one stored
// conversation, "old-1", for session/list and session/resume.
//   anything else sends a plan (one entry with an unknown status), a usage_update, and replies
//   "Done: <task>"
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
const config: Record<string, unknown> = {};
let sessionId = "";
let started = "new";
let cwd = "";
let initialize: unknown;
let authenticated: unknown;

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
    title: "mcp__suite__tools_call",
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
  say(sessionId, text, "agent_thought_chunk");
  say(sessionId, "Looking.");
  if (task.startsWith("greet ")) {
    await greet(sessionId, task.slice(6));
  } else if (task === "ask" || task.startsWith("ask ")) {
    const what = task.slice(4);
    const toolCall: Record<string, unknown> = { toolCallId: "t2", title: "Write notes.txt" };
    const diff = { type: "diff", path: "/tmp/notes.txt", oldText: "a\n", newText: "b\n" };
    if (what === "edit") Object.assign(toolCall, { kind: "edit", content: [diff] });
    else if (what === "diff") Object.assign(toolCall, { content: [diff] });
    else if (what === "execute") Object.assign(toolCall, { kind: "execute", title: "ls" });
    const answer = (await request("session/request_permission", {
      sessionId,
      toolCall,
      options: [
        { optionId: "yes", name: "Allow", kind: "allow_once" },
        { optionId: "no", name: "Reject", kind: "reject_once" },
      ],
    })) as { outcome: { outcome: string; optionId?: string } };
    say(sessionId, "\n\n");
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${answer.outcome.optionId ?? answer.outcome.outcome}`);
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
  } else if (task.startsWith("suitetool ") || task.startsWith("codex ")) {
    const [style, name = ""] = task.split(" ");
    const title = `${style === "codex" ? "mcp.suite." : "mcp__suite__"}${name}`;
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
    say(sessionId, JSON.stringify({ instructions, config, session: sessionId }));
    return { stopReason: "end_turn" };
  } else if (task === "started") {
    say(sessionId, `started: ${started}`);
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
      result = { protocolVersion: 1, agentCapabilities: { mcpCapabilities: { http: true } } };
    } else if (method === "authenticate") {
      authenticated = withHashedHeaders(params);
      result = {};
    } else if (method === "session/new") {
      const servers = params.mcpServers as HttpServer[];
      gateway = servers[0];
      cwd = params.cwd as string;
      const meta = params._meta as { jetbrains?: { air?: { customInstructions?: string } } };
      instructions = meta.jetbrains?.air?.customInstructions?.slice(0, 40);
      sessionId = "acp-1";
      result = { sessionId, configOptions: configOptions() };
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
    } else {
      send({ id, error: { code: -32601, message: `no ${JSON.stringify(method)}` } });
      return;
    }
    if (id !== undefined) send({ id, result });
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

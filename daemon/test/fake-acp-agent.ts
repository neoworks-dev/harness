// A scripted ACP agent (stdio, one JSON-RPC message per line) standing in for Claude Code or
// Codex in guest tests. What it does depends on the task, the prompt's last line:
//   greet <name>  calls greeter.greet through the suite's MCP server (the gateway)
//   ask           asks for permission and reports the answer
//   wait          runs until cancelled
//   edit          reports a tool call with a diff
//   config        reports what session/new and set_config_option gave it
//   anything else replies "Done: <task>"
// Every prompt first echoes the whole prompt text as a thought, so tests can see the event tags.
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
  } else if (task === "ask") {
    const answer = (await request("session/request_permission", {
      sessionId,
      toolCall: { toolCallId: "t2", title: "Write notes.txt" },
      options: [
        { optionId: "yes", name: "Allow", kind: "allow_once" },
        { optionId: "no", name: "Reject", kind: "reject_once" },
      ],
    })) as { outcome: { outcome: string; optionId?: string } };
    say(sessionId, "\n\n");
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, `answer: ${answer.outcome.optionId ?? answer.outcome.outcome}`);
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
  } else if (task === "config") {
    update(sessionId, { sessionUpdate: "plan", entries: [] });
    say(sessionId, JSON.stringify({ instructions, config }));
    return { stopReason: "end_turn" };
  }
  update(sessionId, { sessionUpdate: "plan", entries: [] });
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
      result = { protocolVersion: 1, agentCapabilities: { mcpCapabilities: { http: true } } };
    } else if (method === "session/new") {
      const servers = params.mcpServers as HttpServer[];
      gateway = servers[0];
      const meta = params._meta as { jetbrains?: { air?: { customInstructions?: string } } };
      instructions = meta.jetbrains?.air?.customInstructions?.slice(0, 40);
      result = { sessionId: "acp-1" };
    } else if (method === "session/set_config_option") {
      config[params.configId as string] = params.value;
      result = { configOptions: [] };
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

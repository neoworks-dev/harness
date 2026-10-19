// tools_search, tools_call, apps_launch, apps_focus, delegate, models: native definitions from
// protocol/core-tools.json (so the tool list is fixed whatever is connected), executed by the
// daemon's MCP gateway. The harness is an MCP client of the gateway (design §4).
import type { AgentTool, AgentToolResult } from "../loop/index.ts";
import type { ImageContent, TextContent } from "@mariozechner/pi-ai";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { type CallToolResult, CallToolResultSchema } from "@modelcontextprotocol/sdk/types.js";
import { CORE_TOOLS, type CoreToolName } from "@suite/protocol/core-tools";
import type { McpServer } from "../agent.ts";

export interface GatewayConnection {
  call(
    name: CoreToolName,
    args: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<CallToolResult>;
}

const LABELS: Record<CoreToolName, string> = {
  tools_search: "Search tools",
  tools_call: "Call tool",
  apps_launch: "Launch app",
  apps_focus: "Focus app",
  delegate: "Delegate",
  models: "Models",
};

// A guest can work for a long time; cancelling the turn aborts it through the signal.
const TIMEOUTS: Partial<Record<CoreToolName, number>> = { delegate: 4 * 60 * 60_000 };

/** Connects lazily on first use and again after a failure (the daemon may have restarted). */
export function connectGateway(server: McpServer): GatewayConnection {
  let client: Promise<Client> | undefined;
  const headers = Object.fromEntries(server.headers.map((h) => [h.name, h.value]));

  const connect = async (): Promise<Client> => {
    const c = new Client({ name: "suite-harness", version: "0.2.0" });
    await c.connect(
      new StreamableHTTPClientTransport(new URL(server.url), { requestInit: { headers } }),
    );
    return c;
  };

  return {
    async call(name, args, signal) {
      client ??= connect();
      try {
        const connected = await client;
        return await connected.request(
          { method: "tools/call", params: { name, arguments: args } },
          CallToolResultSchema,
          { signal, timeout: TIMEOUTS[name] ?? 10 * 60_000, resetTimeoutOnProgress: true },
        );
      } catch (err) {
        client = undefined;
        throw err;
      }
    },
  };
}

function asRecord(value: unknown): Record<string, unknown> {
  const record: Record<string, unknown> = {};
  if (typeof value === "object" && value !== null) Object.assign(record, value);
  return record;
}

function toContent(result: CallToolResult): (TextContent | ImageContent)[] {
  const out: (TextContent | ImageContent)[] = [];
  for (const block of result.content) {
    if (block.type === "text") out.push({ type: "text", text: block.text });
    else if (block.type === "image")
      out.push({ type: "image", data: block.data, mimeType: block.mimeType });
    else out.push({ type: "text", text: `[${block.type} content omitted]` });
  }
  return out;
}

/** The core tools. Without a gateway they still exist and answer with an error. */
export function createGatewayTools(gateway: GatewayConnection | undefined): AgentTool[] {
  return CORE_TOOLS.map((def): AgentTool => ({
    name: def.name,
    label: LABELS[def.name],
    description: def.description,
    parameters: def.inputSchema as AgentTool["parameters"],
    async execute(_toolCallId, params, signal): Promise<AgentToolResult<unknown>> {
      if (!gateway) throw new Error("Connectors are unavailable: no gateway is configured.");
      const result = await gateway.call(def.name, asRecord(params), signal);
      const content = toContent(result);
      if (result.isError) {
        // neo turns a thrown error into an error tool result with this text.
        throw new Error(content.map((c) => (c.type === "text" ? c.text : "[image]")).join("\n"));
      }
      return { content, details: result.structuredContent ?? {} };
    },
  }));
}

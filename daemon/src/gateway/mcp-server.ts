// The gateway's MCP endpoint (Streamable HTTP, stateless): serves the fixed core tools to the
// harness, the session's ACP harness and subagents (x-neo-guest: they don't get delegate or
// models; send_message and wait they do).
// x-neo-text marks an ACP client that gets text-only results and, appended to them, the
// messages other agents wrote to it; x-neo-files (the harness's name) one that gets neo's file
// tools, read, edit and write, in place of its own (file-tools.ts). The tool list of a kind of
// client never changes; connector tools are reached through tools_search / tools_call.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  type Tool,
} from "@modelcontextprotocol/sdk/types.js";
import { CORE_TOOLS, isCoreToolName } from "@neoworks-dev/neo-protocol/core-tools";
import { VERSION } from "../version.ts";
import { FILE_TOOL_NAMES, type FileTools } from "./file-tools.ts";
import type { Caller, Gateway } from "./gateway.ts";

function header(
  info: { headers: Record<string, string | string[] | undefined> } | undefined,
  name: string,
): string | undefined {
  const value = info?.headers[name];
  return typeof value === "string" ? value : undefined;
}

function createServer(gateway: Gateway, files?: FileTools): Server {
  const server = new Server(
    { name: "neo", version: VERSION },
    { capabilities: { tools: { listChanged: false } } },
  );
  server.setRequestHandler(ListToolsRequestSchema, (_request, extra) => {
    const tools: Tool[] = CORE_TOOLS.filter(
      ({ name }) =>
        (name !== "delegate" && name !== "models") ||
        header(extra.requestInfo, "x-neo-guest") === undefined,
    ).map(({ name, description, inputSchema }) => ({
      name,
      description,
      inputSchema: { type: "object" as const, ...inputSchema },
    }));
    // x-neo-files (the harness's name): neo's file tools too.
    if (files && header(extra.requestInfo, "x-neo-files") !== undefined) {
      tools.push(...files.list());
    }
    return { tools };
  });
  server.setRequestHandler(CallToolRequestSchema, async (request, extra) => {
    const { name, arguments: args = {} } = request.params;
    const session = header(extra.requestInfo, "x-neo-session");
    const harness = header(extra.requestInfo, "x-neo-files");
    if (files && harness !== undefined && session !== undefined && FILE_TOOL_NAMES.includes(name)) {
      return files.call(name, args, extra.signal, session, harness);
    }
    if (!isCoreToolName(name)) {
      return { content: [{ type: "text", text: `Unknown tool ${name}.` }], isError: true };
    }
    const caller: Caller = {};
    const guest = header(extra.requestInfo, "x-neo-guest");
    if (session !== undefined) caller.session = session;
    if (guest !== undefined) caller.guest = guest;
    if (header(extra.requestInfo, "x-neo-text") !== undefined) caller.text = true;
    return gateway.call(name, args, extra.signal, caller);
  });
  return server;
}

/** Handles one authenticated HTTP request to /mcp; `files` are neo's file tools for x-neo-files. */
export function mcpHandler(
  gateway: Gateway,
  files?: FileTools,
): (req: Request) => Promise<Response> {
  return async (req) => {
    // Stateless: a fresh server and transport per request, nothing to clean up between calls.
    const transport = new WebStandardStreamableHTTPServerTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    });
    const server = createServer(gateway, files);
    await server.connect(transport);
    try {
      return await transport.handleRequest(req);
    } finally {
      void server.close();
    }
  };
}

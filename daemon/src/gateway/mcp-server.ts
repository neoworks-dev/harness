// The gateway's MCP endpoint (Streamable HTTP, stateless): serves the fixed core tools to the
// harness, the session's ACP harness and subagents (x-suite-guest: they don't get delegate or
// models).
// x-suite-text marks an ACP client that gets text-only results. The tool list of a kind of
// client never changes; connector tools are reached through tools_search / tools_call.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { CORE_TOOLS, isCoreToolName } from "@suite/protocol/core-tools";
import { VERSION } from "../version.ts";
import type { Caller, Gateway } from "./gateway.ts";

function header(
  info: { headers: Record<string, string | string[] | undefined> } | undefined,
  name: string,
): string | undefined {
  const value = info?.headers[name];
  return typeof value === "string" ? value : undefined;
}

function createServer(gateway: Gateway): Server {
  const server = new Server(
    { name: "suite", version: VERSION },
    { capabilities: { tools: { listChanged: false } } },
  );
  server.setRequestHandler(ListToolsRequestSchema, (_request, extra) => ({
    tools: CORE_TOOLS.filter(
      ({ name }) =>
        (name !== "delegate" && name !== "models") ||
        header(extra.requestInfo, "x-suite-guest") === undefined,
    ).map(({ name, description, inputSchema }) => ({
      name,
      description,
      inputSchema: { type: "object" as const, ...inputSchema },
    })),
  }));
  server.setRequestHandler(CallToolRequestSchema, async (request, extra) => {
    const { name, arguments: args = {} } = request.params;
    if (!isCoreToolName(name)) {
      return { content: [{ type: "text", text: `Unknown tool ${name}.` }], isError: true };
    }
    const caller: Caller = {};
    const session = header(extra.requestInfo, "x-suite-session");
    const guest = header(extra.requestInfo, "x-suite-guest");
    if (session !== undefined) caller.session = session;
    if (guest !== undefined) caller.guest = guest;
    if (header(extra.requestInfo, "x-suite-text") !== undefined) caller.text = true;
    return gateway.call(name, args, extra.signal, caller);
  });
  return server;
}

/** Handles one authenticated HTTP request to /mcp. */
export function mcpHandler(gateway: Gateway): (req: Request) => Promise<Response> {
  return async (req) => {
    // Stateless: a fresh server and transport per request, nothing to clean up between calls.
    const transport = new WebStandardStreamableHTTPServerTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    });
    const server = createServer(gateway);
    await server.connect(transport);
    try {
      return await transport.handleRequest(req);
    } finally {
      void server.close();
    }
  };
}

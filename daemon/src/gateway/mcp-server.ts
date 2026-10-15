// The gateway's MCP endpoint (Streamable HTTP, stateless): serves the fixed core tools to the
// harness and, later, to guest agents. Its tool list never changes; connector tools are reached
// through tools_search / tools_call.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { CORE_TOOLS, isCoreToolName } from "@suite/protocol/core-tools";
import { VERSION } from "../version.ts";
import type { Gateway } from "./gateway.ts";

function createServer(gateway: Gateway): Server {
  const server = new Server(
    { name: "suite", version: VERSION },
    { capabilities: { tools: { listChanged: false } } },
  );
  server.setRequestHandler(ListToolsRequestSchema, () => ({
    tools: CORE_TOOLS.map(({ name, description, inputSchema }) => ({
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
    return gateway.call(name, args, extra.signal);
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

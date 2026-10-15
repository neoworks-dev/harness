// A minimal app-side MCP server (Streamable HTTP, stateful sessions so list_changed reaches the
// gateway) for gateway tests.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import {
  type CallToolResult,
  CallToolRequestSchema,
  ListToolsRequestSchema,
  type Tool,
} from "@modelcontextprotocol/sdk/types.js";
import type { McpEndpoint } from "@suite/protocol/types";

export interface AppTool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  handler: (args: Record<string, unknown>) => CallToolResult | Promise<CallToolResult>;
}

export interface McpApp {
  endpoint: McpEndpoint;
  calls: { name: string; args: Record<string, unknown> }[];
  setTools(tools: AppTool[]): Promise<void>;
  close(): Promise<void>;
}

export function startMcpApp(options: { tools: AppTool[]; instructions?: string }): McpApp {
  let tools = options.tools;
  const calls: McpApp["calls"] = [];
  const sessions = new Map<
    string,
    { server: Server; transport: WebStandardStreamableHTTPServerTransport }
  >();
  const token = crypto.randomUUID();

  const createServer = (): Server => {
    const server = new Server(
      { name: "test-app", version: "1.0.0" },
      { capabilities: { tools: { listChanged: true } }, instructions: options.instructions },
    );
    server.setRequestHandler(ListToolsRequestSchema, () => ({
      tools: tools.map(({ name, description, inputSchema, outputSchema }) => {
        const listed: Tool = { name, description, inputSchema: { type: "object", ...inputSchema } };
        if (outputSchema) listed.outputSchema = { type: "object", ...outputSchema };
        return listed;
      }),
    }));
    server.setRequestHandler(CallToolRequestSchema, async (req) => {
      const args = req.params.arguments ?? {};
      calls.push({ name: req.params.name, args });
      const tool = tools.find((t) => t.name === req.params.name);
      if (!tool) return { content: [{ type: "text", text: "no such tool" }], isError: true };
      return tool.handler(args);
    });
    return server;
  };

  const http = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    idleTimeout: 0,
    async fetch(req) {
      if (req.headers.get("authorization") !== `Bearer ${token}`) {
        return new Response("unauthorized", { status: 401 });
      }
      const id = req.headers.get("mcp-session-id");
      if (id !== null) {
        const existing = sessions.get(id);
        if (!existing) return new Response("unknown session", { status: 404 });
        return existing.transport.handleRequest(req);
      }
      const server = createServer();
      const transport = new WebStandardStreamableHTTPServerTransport({
        sessionIdGenerator: () => crypto.randomUUID(),
        onsessioninitialized: (sid) => {
          sessions.set(sid, { server, transport });
        },
      });
      await server.connect(transport);
      return transport.handleRequest(req);
    },
  });

  return {
    endpoint: {
      url: `http://127.0.0.1:${http.port}/mcp`,
      headers: { authorization: `Bearer ${token}` },
    },
    calls,
    async setTools(next) {
      tools = next;
      await Promise.all([...sessions.values()].map((s) => s.server.sendToolListChanged()));
    },
    async close() {
      await Promise.all([...sessions.values()].map((s) => s.server.close()));
      await http.stop(true);
    },
  };
}

// Hosts an app's tools as an MCP server (Streamable HTTP) on loopback, behind a random bearer
// token. The app hands `endpoint` to the daemon in its presence; the gateway lists the tools and
// forwards calls here.
import { randomUUID, timingSafeEqual } from "node:crypto";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import {
  type CallToolResult,
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import type { McpEndpoint } from "@suite/protocol/types";

export type ToolContent = CallToolResult["content"];

export interface ToolResult {
  content?: ToolContent;
  structuredContent?: Record<string, unknown>;
  isError?: boolean;
}

export interface ToolSpec {
  name: string;
  description: string;
  /** JSON Schema of the arguments (an object schema). */
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  /** A string becomes one text block. A thrown error becomes an `isError` text result. */
  handler(args: Record<string, unknown>): Promise<string | ToolResult>;
}

export interface ToolServerOptions {
  name: string;
  version?: string;
  /** Goes into the MCP initialize result; the gateway can show it next to the app's tools. */
  instructions?: string;
  tools: ToolSpec[];
}

export interface ToolServer {
  /** The MCP endpoint, e.g. http://127.0.0.1:41234/mcp */
  url: string;
  /** Headers every request must carry (the bearer token). */
  headers: Record<string, string>;
  /** `{ url, headers }`, ready for `presence.mcp`. */
  endpoint: McpEndpoint;
  /** Replaces the tool list and tells connected clients with tools/list_changed. */
  setTools(tools: ToolSpec[]): void;
  close(): Promise<void>;
}

interface McpSession {
  server: Server;
  transport: WebStandardStreamableHTTPServerTransport;
}

function sameToken(given: string | null, token: string): boolean {
  if (given === null || given.length !== token.length) return false;
  return timingSafeEqual(Buffer.from(given), Buffer.from(token));
}

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

function listed(spec: ToolSpec): Record<string, unknown> {
  const { name, description, inputSchema, outputSchema } = spec;
  const tool: Record<string, unknown> = { name, description, inputSchema };
  if (outputSchema) tool.outputSchema = outputSchema;
  return tool;
}

export function createToolServer(options: ToolServerOptions): ToolServer {
  let tools = options.tools;
  const sessions = new Map<string, McpSession>();
  const token = Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString("hex");

  const createServer = (): Server => {
    const server = new Server(
      { name: options.name, version: options.version ?? "0.1.0" },
      { capabilities: { tools: { listChanged: true } }, instructions: options.instructions },
    );
    server.setRequestHandler(ListToolsRequestSchema, () => ({ tools: tools.map(listed) }));
    server.setRequestHandler(CallToolRequestSchema, async (call): Promise<CallToolResult> => {
      const spec = tools.find((tool) => tool.name === call.params.name);
      if (!spec) {
        return { isError: true, content: [{ type: "text", text: `no tool ${call.params.name}` }] };
      }
      try {
        const result = await spec.handler(call.params.arguments ?? {});
        if (typeof result === "string") return { content: [{ type: "text", text: result }] };
        return { ...result, content: result.content ?? [] };
      } catch (err) {
        return { isError: true, content: [{ type: "text", text: errorText(err) }] };
      }
    });
    return server;
  };

  const handleMcp = async (req: Request): Promise<Response> => {
    const sessionId = req.headers.get("mcp-session-id");
    if (sessionId !== null) {
      const existing = sessions.get(sessionId);
      if (!existing) {
        const error = { code: -32001, message: "Session not found" };
        return Response.json({ jsonrpc: "2.0", error, id: null }, { status: 404 });
      }
      return existing.transport.handleRequest(req);
    }
    const server = createServer();
    const transport = new WebStandardStreamableHTTPServerTransport({
      sessionIdGenerator: () => randomUUID(),
      onsessioninitialized: (id) => {
        sessions.set(id, { server, transport });
      },
      onsessionclosed: (id) => {
        sessions.delete(id);
      },
    });
    await server.connect(transport);
    return transport.handleRequest(req);
  };

  const http = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    idleTimeout: 0, // the standalone SSE stream stays open
    fetch(req) {
      if (new URL(req.url).pathname !== "/mcp") return new Response("not found", { status: 404 });
      const header = req.headers.get("authorization");
      const given = header?.startsWith("Bearer ") ? header.slice("Bearer ".length) : null;
      if (!sameToken(given, token)) return new Response("unauthorized", { status: 401 });
      return handleMcp(req);
    },
  });

  const url = `http://127.0.0.1:${http.port}/mcp`;
  const headers = { Authorization: `Bearer ${token}` };
  return {
    url,
    headers,
    endpoint: { url, headers },
    setTools(next) {
      tools = next;
      for (const { server } of sessions.values()) {
        server.sendToolListChanged().catch(() => {});
      }
    },
    async close() {
      await Promise.all([...sessions.values()].map(({ server }) => server.close()));
      sessions.clear();
      await http.stop(true);
    },
  };
}

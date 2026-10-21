import { randomUUID } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";

interface McpSession {
  server: Server;
  transport: StreamableHTTPServerTransport;
}

const SESSION_NOT_FOUND = { code: -32001, message: "Session not found" };

/**
 * The MCP sessions of a tool server. A client that sends no session id starts a new one (with
 * its own MCP server); one that does is routed to its session.
 */
export class McpSessions {
  private readonly sessions = new Map<string, McpSession>();

  constructor(private readonly createServer: () => Server) {}

  handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const id = req.headers["mcp-session-id"];
    if (typeof id !== "string") return this.startSession(req, res);
    return this.continueSession(id, req, res);
  }

  /** Tells every connected client that the tool list changed. */
  announceToolChange(): void {
    for (const { server } of this.sessions.values()) {
      server.sendToolListChanged().catch(() => {});
    }
  }

  async close(): Promise<void> {
    await Promise.all([...this.sessions.values()].map(({ server }) => server.close()));
    this.sessions.clear();
  }

  private async startSession(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const server = this.createServer();
    const transport: StreamableHTTPServerTransport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => randomUUID(),
      onsessioninitialized: (id) => {
        this.sessions.set(id, { server, transport });
      },
      onsessionclosed: (id) => {
        this.sessions.delete(id);
      },
    });
    await server.connect(transport);
    await transport.handleRequest(req, res);
  }

  private async continueSession(
    id: string,
    req: IncomingMessage,
    res: ServerResponse,
  ): Promise<void> {
    const session = this.sessions.get(id);
    if (!session) {
      const body = { jsonrpc: "2.0", error: SESSION_NOT_FOUND, id: null };
      res.writeHead(404, { "content-type": "application/json" }).end(JSON.stringify(body));
      return;
    }
    await session.transport.handleRequest(req, res);
  }
}

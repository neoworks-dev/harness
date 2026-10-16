import { randomUUID } from "node:crypto";
import type { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";

interface McpSession {
  server: Server;
  transport: WebStandardStreamableHTTPServerTransport;
}

const SESSION_NOT_FOUND = { code: -32001, message: "Session not found" };

/**
 * The MCP sessions of a tool server. A client that sends no session id starts a new one (with
 * its own MCP server); one that does is routed to its session.
 */
export class McpSessions {
  private readonly sessions = new Map<string, McpSession>();

  constructor(private readonly createServer: () => Server) {}

  handle(req: Request): Promise<Response> {
    const id = req.headers.get("mcp-session-id");
    if (id === null) return this.startSession(req);
    return this.continueSession(id, req);
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

  private async startSession(req: Request): Promise<Response> {
    const server = this.createServer();
    const transport: WebStandardStreamableHTTPServerTransport =
      new WebStandardStreamableHTTPServerTransport({
        sessionIdGenerator: () => randomUUID(),
        onsessioninitialized: (id) => {
          this.sessions.set(id, { server, transport });
        },
        onsessionclosed: (id) => {
          this.sessions.delete(id);
        },
      });
    await server.connect(transport);
    return transport.handleRequest(req);
  }

  private continueSession(id: string, req: Request): Promise<Response> {
    const session = this.sessions.get(id);
    if (!session) {
      const body = { jsonrpc: "2.0", error: SESSION_NOT_FOUND, id: null };
      return Promise.resolve(Response.json(body, { status: 404 }));
    }
    return session.transport.handleRequest(req);
  }
}

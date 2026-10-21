// Hosts an app's tools as an MCP server (Streamable HTTP) on loopback, behind a random bearer
// token. The app hands `endpoint` to the daemon in its presence; the daemon's gateway lists the
// tools and forwards the agent's calls here. node:http, so it runs in Node and Bun alike.
import { createServer } from "node:http";
import { hasToken, newToken } from "./authorize.ts";
import { createMcpServer } from "./mcp-server.ts";
import { McpSessions } from "./mcp-sessions.ts";
import type { ToolServer, ToolServerOptions, ToolSpec } from "./types.ts";

const MCP_PATH = "/mcp";

/** Starts the server on a free port of 127.0.0.1. */
export async function createToolServer(options: ToolServerOptions): Promise<ToolServer> {
  let tools = options.tools;
  const token = newToken();
  const sessions = new McpSessions(() => createMcpServer(options, () => tools));

  const http = createServer((req, res) => {
    const path = new URL(req.url ?? "/", "http://127.0.0.1").pathname;
    if (path !== MCP_PATH) {
      res.writeHead(404).end("not found");
      return;
    }
    if (!hasToken(req.headers.authorization, token)) {
      res.writeHead(401).end("unauthorized");
      return;
    }
    sessions.handle(req, res).catch((err: unknown) => {
      if (!res.headersSent) res.writeHead(500);
      res.end(err instanceof Error ? err.message : String(err));
    });
  });
  // The standalone SSE stream stays open: no request or idle timeouts.
  http.requestTimeout = 0;
  http.keepAliveTimeout = 0;
  http.timeout = 0;
  await new Promise<void>((done, fail) => {
    http.once("error", fail);
    http.listen(0, "127.0.0.1", () => done());
  });

  const address = http.address();
  if (address === null || typeof address === "string")
    throw new Error("the tool server has no port");
  const { port } = address;
  const url = `http://127.0.0.1:${port}${MCP_PATH}`;
  const headers = { Authorization: `Bearer ${token}` };
  return {
    url,
    headers,
    endpoint: { url, headers },
    setTools(next: ToolSpec[]) {
      tools = next;
      sessions.announceToolChange();
    },
    async close() {
      await sessions.close();
      http.closeAllConnections();
      await new Promise<void>((done) => http.close(() => done()));
    },
  };
}

// Hosts an app's tools as an MCP server (Streamable HTTP) on loopback, behind a random bearer
// token. The app hands `endpoint` to the daemon in its presence; the daemon's gateway lists the
// tools and forwards the agent's calls here.
import { hasToken, newToken } from "./authorize.ts";
import { createMcpServer } from "./mcp-server.ts";
import { McpSessions } from "./mcp-sessions.ts";
import type { ToolServer, ToolServerOptions, ToolSpec } from "./types.ts";

const MCP_PATH = "/mcp";

/** Starts the server on a free port of 127.0.0.1. */
export function createToolServer(options: ToolServerOptions): ToolServer {
  let tools = options.tools;
  const token = newToken();
  const sessions = new McpSessions(() => createMcpServer(options, () => tools));

  const http = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    idleTimeout: 0, // the standalone SSE stream stays open
    fetch(req) {
      if (new URL(req.url).pathname !== MCP_PATH) return new Response("not found", { status: 404 });
      if (!hasToken(req, token)) return new Response("unauthorized", { status: 401 });
      return sessions.handle(req);
    },
  });

  const url = `http://127.0.0.1:${http.port}${MCP_PATH}`;
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
      await http.stop(true);
    },
  };
}

// The gateway's MCP client side: one connection per connected app's MCP server.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import {
  type CallToolResult,
  CallToolResultSchema,
  ToolListChangedNotificationSchema,
} from "@modelcontextprotocol/sdk/types.js";
import type { McpEndpoint } from "@suite/protocol/types";
import type { StoredTool } from "../store.ts";

export interface Upstream {
  listTools(): Promise<StoredTool[]>;
  instructions(): string | undefined;
  call(name: string, args: Record<string, unknown>, signal?: AbortSignal): Promise<CallToolResult>;
  /** Called when the app announces that its tool list changed. */
  onToolsChanged(listener: () => void): void;
  close(): Promise<void>;
}

export type ConnectUpstream = (endpoint: McpEndpoint) => Promise<Upstream>;

const TIMEOUT_MS = 10_000;

export const connectMcp: ConnectUpstream = async (endpoint) => {
  const client = new Client({ name: "suite-gateway", version: "0.2.0" });
  const transport = new StreamableHTTPClientTransport(new URL(endpoint.url), {
    requestInit: { headers: endpoint.headers ?? {} },
  });
  await client.connect(transport, { timeout: TIMEOUT_MS });
  const listeners: (() => void)[] = [];
  client.setNotificationHandler(ToolListChangedNotificationSchema, () => {
    for (const listener of listeners) listener();
  });

  return {
    async listTools() {
      const tools: StoredTool[] = [];
      let cursor: string | undefined;
      do {
        const page = await client.listTools(cursor === undefined ? undefined : { cursor }, {
          timeout: TIMEOUT_MS,
        });
        for (const tool of page.tools) {
          const stored: StoredTool = {
            name: tool.name,
            description: tool.description ?? "",
            inputSchema: tool.inputSchema,
          };
          if (tool.outputSchema) stored.outputSchema = tool.outputSchema;
          tools.push(stored);
        }
        cursor = page.nextCursor;
      } while (cursor !== undefined);
      return tools;
    },
    instructions: () => client.getInstructions(),
    async call(name, args, signal) {
      // No client-side output validation: the result is passed through as the app sent it.
      const result = await client.request(
        { method: "tools/call", params: { name, arguments: args } },
        CallToolResultSchema,
        { signal, timeout: 5 * 60_000, resetTimeoutOnProgress: true },
      );
      return result;
    },
    onToolsChanged(listener) {
      listeners.push(listener);
    },
    async close() {
      await client.close().catch(() => undefined);
    },
  };
};

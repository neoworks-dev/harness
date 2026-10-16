import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import {
  type CallToolResult,
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import type { ToolServerOptions, ToolSpec } from "./types.ts";

/** What `tools/list` says about a tool: the spec without its handler. */
function describe(spec: ToolSpec): Record<string, unknown> {
  const { name, description, inputSchema, outputSchema } = spec;
  const listed: Record<string, unknown> = { name, description, inputSchema };
  if (outputSchema) listed.outputSchema = outputSchema;
  return listed;
}

function errorMessage(thrown: unknown): string {
  if (thrown instanceof Error) return thrown.message;
  return String(thrown);
}

function failure(text: string): CallToolResult {
  return { isError: true, content: [{ type: "text", text }] };
}

async function runTool(spec: ToolSpec, args: Record<string, unknown>): Promise<CallToolResult> {
  try {
    const result = await spec.handler(args);
    if (typeof result === "string") return { content: [{ type: "text", text: result }] };
    return { ...result, content: result.content ?? [] };
  } catch (thrown) {
    return failure(errorMessage(thrown));
  }
}

/**
 * One MCP server for one client session. It looks the tools up through `currentTools` on every
 * call, so a changed tool list takes effect without reconnecting.
 */
export function createMcpServer(
  options: Pick<ToolServerOptions, "name" | "version" | "instructions">,
  currentTools: () => ToolSpec[],
): Server {
  const server = new Server(
    { name: options.name, version: options.version ?? "0.1.0" },
    { capabilities: { tools: { listChanged: true } }, instructions: options.instructions },
  );
  server.setRequestHandler(ListToolsRequestSchema, () => ({ tools: currentTools().map(describe) }));
  server.setRequestHandler(CallToolRequestSchema, async (call): Promise<CallToolResult> => {
    const spec = currentTools().find((tool) => tool.name === call.params.name);
    if (!spec) return failure(`no tool ${call.params.name}`);
    return runTool(spec, call.params.arguments ?? {});
  });
  return server;
}

// The gateway's MCP client side: one connection per connected app's MCP server, over Streamable
// HTTP (an app's endpoint) or stdio (a program the daemon started, see mcp-servers.ts).
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import {
  getDefaultEnvironment,
  type StdioServerParameters,
  StdioClientTransport,
} from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import {
  type CallToolResult,
  CallToolResultSchema,
  ToolListChangedNotificationSchema,
} from "@modelcontextprotocol/sdk/types.js";
import type { McpEndpoint } from "@neoworks-dev/neo-protocol/types";
import type { StoredSkill, StoredTool } from "../store.ts";

export interface Upstream {
  listTools(): Promise<StoredTool[]>;
  /** Its skills: the prompts it offers that take no arguments, with their text. */
  listSkills(): Promise<StoredSkill[]>;
  instructions(): string | undefined;
  call(name: string, args: Record<string, unknown>, signal?: AbortSignal): Promise<CallToolResult>;
  /** Called when the app announces that its tool list changed. */
  onToolsChanged(listener: () => void): void;
  close(): Promise<void>;
}

export type ConnectUpstream = (endpoint: McpEndpoint) => Promise<Upstream>;

const TIMEOUT_MS = 10_000;

export const connectMcp: ConnectUpstream = async (endpoint) => {
  const client = new Client({ name: "neo-gateway", version: "0.2.0" });
  const transport = new StreamableHTTPClientTransport(new URL(endpoint.url), {
    requestInit: { headers: endpoint.headers ?? {} },
  });
  await client.connect(transport, { timeout: TIMEOUT_MS });
  return upstreamOf(client);
};

/** What a stdio MCP server is started with (see neo/src/mcp-config.ts). */
export interface StdioSpec {
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd?: string;
}

/** Variables besides the SDK's defaults (HOME, PATH, …) a stdio server is started with. */
const PASSED_ENV = [
  "LANG",
  "LC_ALL",
  "TMPDIR",
  "XDG_CONFIG_HOME",
  "XDG_DATA_HOME",
  "XDG_CACHE_HOME",
  "XDG_RUNTIME_DIR",
  "SSL_CERT_FILE",
];

/** A stdio server running, and its end of the MCP connection. */
export interface StdioConnection {
  upstream: Upstream;
  /** Calls `listener` once, when the process or the connection is gone (not after `close`). */
  onExit(listener: (reason: string) => void): void;
}

/**
 * Starts the program and connects to it. Rejects when it can't start, or doesn't speak MCP within
 * the timeout (the process is stopped then). The environment is the SDK's small default, a few
 * more variables and the spec's own: the daemon's secrets stay out of servers that never asked.
 */
export async function connectStdio(spec: StdioSpec): Promise<StdioConnection> {
  const env: Record<string, string> = { ...getDefaultEnvironment() };
  for (const name of PASSED_ENV) {
    const value = process.env[name];
    if (value !== undefined) env[name] = value;
  }
  Object.assign(env, spec.env);
  const params: StdioServerParameters = {
    command: spec.command,
    args: spec.args,
    env,
    stderr: "pipe",
  };
  if (spec.cwd !== undefined) params.cwd = spec.cwd;
  const transport = new StdioClientTransport(params);
  let tail = "";
  transport.stderr?.on("data", (chunk: Buffer) => {
    tail = (tail + chunk.toString()).slice(-2000);
  });
  const client = new Client({ name: "neo-gateway", version: "0.2.0" });
  const exits: ((reason: string) => void)[] = [];
  let closing = false;
  let exited = false;
  const exit = (): void => {
    if (exited || closing) return;
    exited = true;
    for (const listener of exits) listener("the process exited");
  };
  transport.onclose = exit;
  try {
    await client.connect(transport, { timeout: TIMEOUT_MS });
  } catch (err) {
    closing = true;
    await client.close().catch(() => undefined);
    const message = err instanceof Error ? err.message : String(err);
    const detail = tail.trim().split("\n").slice(-3).join(" | ");
    throw new Error(detail === "" ? message : `${message} (${detail})`);
  }
  const upstream = upstreamOf(client, () => {
    closing = true;
  });
  return {
    upstream,
    onExit(listener) {
      exits.push(listener);
      if (exited) listener("the process exited");
    },
  };
}

/** The gateway's view of a connected MCP client; `closing` runs before the client is closed. */
function upstreamOf(client: Client, closing?: () => void): Upstream {
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
    async listSkills() {
      if (client.getServerCapabilities()?.prompts === undefined) return [];
      const skills: StoredSkill[] = [];
      let cursor: string | undefined;
      do {
        const page = await client.listPrompts(cursor === undefined ? undefined : { cursor }, {
          timeout: TIMEOUT_MS,
        });
        for (const prompt of page.prompts) {
          // One that takes arguments is a template to fill in, not instructions to read.
          if ((prompt.arguments ?? []).some((a) => a.required === true)) continue;
          const got = await client.getPrompt({ name: prompt.name }, { timeout: TIMEOUT_MS });
          const parts: string[] = [];
          for (const m of got.messages) {
            if (m.content.type === "text" && m.content.text.trim() !== "")
              parts.push(m.content.text);
          }
          const text = parts.join("\n\n");
          let description = prompt.description;
          if (description === undefined) description = got.description;
          description = description?.trim();
          if (text === "" || description === undefined || description === "") continue;
          skills.push({ name: prompt.name, description, text });
        }
        cursor = page.nextCursor;
      } while (cursor !== undefined);
      return skills;
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
      closing?.();
      await client.close().catch(() => undefined);
    },
  };
}

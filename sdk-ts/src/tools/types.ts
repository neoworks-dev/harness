import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { McpEndpoint } from "@neoworks-dev/neo-protocol/types";

/** The content blocks of a tool result: text, images and so on, as MCP defines them. */
export type ToolContent = CallToolResult["content"];

/** What a tool handler may return instead of a plain string. */
export interface ToolResult {
  /** What the agent reads. */
  content?: ToolContent;
  /** The same result as data. The app's templates draw it in the chat. */
  structuredContent?: Record<string, unknown>;
  /** Marks the result as a failure the agent should react to. */
  isError?: boolean;
}

/**
 * MCP's hints about a tool. The daemon decides from them whether the agent's call asks the user
 * first: a read never asks, an edit the app can undo asks only in ask mode, and any other tool
 * asks in every mode but auto.
 */
export interface ToolAnnotations {
  /** The tool changes nothing. */
  readOnlyHint?: boolean;
  /** `false`: the tool's changes can be taken back in the app (its own undo). Default `true`. */
  destructiveHint?: boolean;
}

/** One tool the app offers to the agent. */
export interface ToolSpec {
  /** The tool's name; the daemon prefixes it with the app name (`notes.append`). */
  name: string;
  /** Tells the agent what the tool does and when to use it. */
  description: string;
  /** JSON Schema of the arguments (an object schema). */
  inputSchema: Record<string, unknown>;
  /** JSON Schema of `structuredContent`, if the tool returns one. */
  outputSchema?: Record<string, unknown>;
  annotations?: ToolAnnotations;
  /**
   * Runs the tool. A returned string becomes one text block. A thrown error becomes a result
   * with `isError` set and the error's message as its text, so the agent sees what went wrong.
   */
  handler(args: Record<string, unknown>): Promise<string | ToolResult>;
}

export interface ToolServerOptions {
  /** The app name. */
  name: string;
  /** Version reported to MCP clients (default 0.1.0). */
  version?: string;
  /** Tells the agent how to use the tools together; the gateway shows it with the app's tools. */
  instructions?: string;
  tools: ToolSpec[];
}

/** An MCP server on this machine's loopback address, for the daemon's gateway to call. */
export interface ToolServer {
  /** The MCP endpoint, for example `http://127.0.0.1:41234/mcp`. */
  url: string;
  /** Headers every request must carry (the bearer token). */
  headers: Record<string, string>;
  /** `{ url, headers }`, ready to use as `presence.mcp`. */
  endpoint: McpEndpoint;
  /** Replaces the tool list and tells connected clients that it changed. */
  setTools(tools: ToolSpec[]): void;
  /** Stops the server and ends its MCP sessions. */
  close(): Promise<void>;
}

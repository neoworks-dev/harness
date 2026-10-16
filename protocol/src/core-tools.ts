// The connector-facing core tools (tools_search, tools_call, apps_launch, apps_focus, delegate),
// shared by the harness (native definitions) and the MCP gateway (which implements them).
import data from "../core-tools.json";

export interface CoreToolDefinition {
  name: CoreToolName;
  description: string;
  inputSchema: Record<string, unknown>;
}

export type CoreToolName =
  "tools_search" | "tools_call" | "apps_launch" | "apps_focus" | "delegate";

export interface ToolsSearchArgs {
  query: string;
  app?: string;
}

export interface ToolsCallArgs {
  name: string;
  args?: Record<string, unknown>;
}

export interface AppArgs {
  app: string;
}

export interface DelegateArgs {
  agent: string;
  task: string;
  model?: string;
  effort?: string;
  cwd?: string;
  worktree?: string;
}

export const CORE_TOOLS: readonly CoreToolDefinition[] = data.tools.map((tool) => {
  if (!isCoreToolName(tool.name)) throw new Error(`core-tools.json: unexpected tool ${tool.name}`);
  return { name: tool.name, description: tool.description, inputSchema: tool.inputSchema };
});

export function isCoreToolName(name: string): name is CoreToolName {
  return ["tools_search", "tools_call", "apps_launch", "apps_focus", "delegate"].includes(name);
}

// neo's file tools on their own: hashline read, edit and write, for a harness that works with
// them in place of its own (the daemon serves them to Claude Code over MCP).
import { validateToolArguments } from "@mariozechner/pi-ai";
import type { RequestPermission } from "./agent.ts";
import type { AgentTool, AgentToolResult } from "./loop/index.ts";
import { createEditTool } from "./tools/edit.ts";
import type { AssetResolver } from "./tools/paths.ts";
import { createReadTool } from "./tools/read.ts";
import { createWriteTool, type FileChangeDetails } from "./tools/write.ts";

export type { AgentTool, AgentToolResult, FileChangeDetails };

/** read, edit and write for `cwd`, asking through `ask` (when given) as they do for neo. */
export function createFileTools(
  cwd: string,
  assets?: AssetResolver,
  ask?: RequestPermission,
): AgentTool[] {
  return [createReadTool(cwd, assets, ask), createEditTool(cwd, ask), createWriteTool(cwd, ask)];
}

/** Runs one of them with arguments a model wrote: checked against its parameters first. */
export function runFileTool(
  tool: AgentTool,
  toolCallId: string,
  args: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<AgentToolResult<unknown>> {
  const checked = validateToolArguments(tool, {
    type: "toolCall",
    id: toolCallId,
    name: tool.name,
    arguments: args,
  });
  return tool.execute(toolCallId, checked, signal);
}

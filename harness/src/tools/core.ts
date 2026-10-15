import type { AgentState } from "@mariozechner/pi-agent-core";
import { createBashTool } from "./bash.ts";
import { createEditTool } from "./edit.ts";
import { createReadTool } from "./read.ts";
import { createWriteTool } from "./write.ts";

/** The fixed core tool list. Order and definitions never change within a session. */
export function createCoreTools(cwd: string): AgentState["tools"] {
  return [createBashTool(cwd), createReadTool(cwd), createWriteTool(cwd), createEditTool(cwd)];
}

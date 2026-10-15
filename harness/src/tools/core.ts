import type { AgentState } from "@mariozechner/pi-agent-core";
import { createBashTool } from "./bash.ts";
import { createEditTool } from "./edit.ts";
import { createReadTool } from "./read.ts";
import { createWriteTool } from "./write.ts";
import { createGatewayTools, type GatewayConnection } from "./gateway.ts";

/** The fixed core tool list. Order and definitions never change, whatever is connected. */
export function createCoreTools(
  cwd: string,
  gateway: GatewayConnection | undefined,
): AgentState["tools"] {
  return [
    createBashTool(cwd),
    createReadTool(cwd),
    createWriteTool(cwd),
    createEditTool(cwd),
    ...createGatewayTools(gateway),
  ];
}

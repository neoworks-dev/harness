import type { AgentState } from "@mariozechner/pi-agent-core";
import { createBashTool } from "./bash.ts";
import { createEditTool } from "./edit.ts";
import { createReadTool } from "./read.ts";
import { createTodoWriteTool } from "./todo.ts";
import { createWriteTool } from "./write.ts";
import { createGatewayTools, type GatewayConnection } from "./gateway.ts";
import type { AssetResolver } from "./paths.ts";

/** The fixed core tool list. Order and definitions never change, whatever is connected. */
export function createCoreTools(
  cwd: string,
  gateway: GatewayConnection | undefined,
  assets?: AssetResolver,
): AgentState["tools"] {
  return [
    createBashTool(cwd, assets),
    createReadTool(cwd, assets),
    createWriteTool(cwd),
    createEditTool(cwd),
    createTodoWriteTool(),
    ...createGatewayTools(gateway),
  ];
}

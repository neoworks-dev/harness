import type { AgentState } from "../loop/index.ts";
import { createBashTool } from "./bash.ts";
import { createEditTool } from "./edit.ts";
import { createReadTool } from "./read.ts";
import { createTodoWriteTool } from "./todo.ts";
import { createWriteTool } from "./write.ts";
import { createGatewayTools, type GatewayConnection } from "./gateway.ts";
import type { RequestPermission } from "../agent.ts";
import type { AssetResolver } from "./paths.ts";

/**
 * The fixed core tool list. Order and definitions never change, whatever is connected. `ask`
 * (when given) is consulted before bash, read, write and edit act.
 */
export function createCoreTools(
  cwd: string,
  gateway: GatewayConnection | undefined,
  assets?: AssetResolver,
  ask?: RequestPermission,
): AgentState["tools"] {
  return [
    createBashTool(cwd, assets, ask),
    createReadTool(cwd, assets, ask),
    createWriteTool(cwd, ask),
    createEditTool(cwd, ask),
    createTodoWriteTool(),
    ...createGatewayTools(gateway),
  ];
}

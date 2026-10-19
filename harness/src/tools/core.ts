import type { AgentState } from "../loop/index.ts";
import { createBashTool } from "./bash.ts";
import { createEditTool } from "./edit.ts";
import { createReadTool } from "./read.ts";
import { createTodoWriteTool } from "./todo.ts";
import { createWriteTool } from "./write.ts";
import { createGatewayTools, type GatewayConnection } from "./gateway.ts";
import type { RequestPermission } from "../agent.ts";
import type { AssetResolver } from "./paths.ts";
import { createWebTools, type WebToolsOptions } from "./web.ts";

/**
 * The fixed core tool list. Order and definitions never change, whatever is connected. `ask`
 * (when given) is consulted before bash, read, write, edit and web_fetch act. Without `web`
 * there are no web tools (tests that don't need them).
 */
export function createCoreTools(
  cwd: string,
  gateway: GatewayConnection | undefined,
  assets?: AssetResolver,
  ask?: RequestPermission,
  web?: WebToolsOptions,
): AgentState["tools"] {
  const tools: AgentState["tools"] = [
    createBashTool(cwd, assets, ask),
    createReadTool(cwd, assets, ask),
    createWriteTool(cwd, ask),
    createEditTool(cwd, ask),
    createTodoWriteTool(),
  ];
  if (web) tools.push(...createWebTools(web, ask));
  tools.push(...createGatewayTools(gateway));
  return tools;
}

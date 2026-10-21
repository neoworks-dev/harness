// What an app's tool is, for approvals (issue #34), from MCP's standard annotations in tools/list:
//   readOnlyHint: true      a read: never asks
//   destructiveHint: false  an edit the app can undo (its own undo): asks in ask mode only
//   neither                 an edit that may not be undoable: asks in every mode but auto
import type { StoredTool } from "../store.ts";

export type AppToolKind = "read" | "undoable" | "other";

/** An agent's call to an app's tool. */
export interface AppCall {
  session: string;
  /** Who calls: a harness name, shown on the cards. */
  agent: string;
  app: string;
  /** The tool's name in the app (without the app prefix). */
  tool: string;
  args: Record<string, unknown>;
}

export function appToolKind(tool: StoredTool): AppToolKind {
  if (tool.annotations?.readOnlyHint === true) return "read";
  return tool.annotations?.destructiveHint === false ? "undoable" : "other";
}

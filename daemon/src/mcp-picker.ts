// /mcp as a picker for an agent whose adapter manages its own MCP servers (Claude Code's): the
// servers with their status, one option each, and picking one acts on it, the choice staying
// open: a disabled server is enabled, a connected one disabled, any other reconnected. The
// adapter only answers `/mcp`, `/mcp enable <server>`… with markdown (its formatMcpStatus), so
// that is run without logging (GuestManager.quietCommand) and read back.
import type { Choice } from "@neoworks-dev/neo-protocol/types";

export type McpStatus = "connected" | "failed" | "needs-auth" | "pending" | "disabled";

export interface McpServer {
  name: string;
  status: McpStatus;
  /** Its error or tool count, as the adapter says. */
  detail?: string;
}

/** The adapter's group titles. */
const GROUPS: Record<string, McpStatus> = {
  Connected: "connected",
  Failed: "failed",
  "Needs authentication": "needs-auth",
  Connecting: "pending",
};

/** What a status reads as, and what picking a server in it does. */
const SHOWN: Record<McpStatus, { label: string; action: McpAction }> = {
  connected: { label: "connected", action: "disable" },
  failed: { label: "failed", action: "reconnect" },
  "needs-auth": { label: "needs authentication", action: "reconnect" },
  pending: { label: "connecting", action: "reconnect" },
  disabled: { label: "disabled", action: "enable" },
};

export type McpAction = "enable" | "disable" | "reconnect";

/** `name` out of the adapter's code span (`` `name` ``, maybe with its origin after it). */
function unquote(text: string): string {
  const match = /^(`+) ?(.*?) ?\1/.exec(text.trim());
  return match?.[2] ?? text.trim();
}

/** The servers in the adapter's /mcp markdown; empty when it lists none (or isn't that). */
export function parseMcpStatus(markdown: string): McpServer[] {
  const servers: McpServer[] = [];
  let group: McpStatus | undefined;
  for (const raw of markdown.split("\n")) {
    const line = raw.trim();
    const disabled = /^\*\*Disabled:\*\*\s*(.*)$/.exec(line);
    if (disabled) {
      for (const name of (disabled[1] ?? "").split(/,\s*(?=`)/)) {
        if (name.trim() !== "") servers.push({ name: unquote(name), status: "disabled" });
      }
      group = undefined;
      continue;
    }
    const title = /^\*\*(.+)\*\*$/.exec(line);
    if (title) {
      group = GROUPS[title[1] ?? ""];
      continue;
    }
    if (group === undefined || !line.startsWith("- ")) continue;
    const item = line.slice(2);
    const span = /^(`+) ?.*? ?\1/.exec(item);
    if (!span) continue;
    const name = unquote(span[0]);
    const rest = item.slice(span[0].length);
    const server: McpServer = { name, status: group };
    const detail = rest.replace(/^\s*(\([^)]*\))?\s*:?\s*/, "").trim();
    if (detail !== "") server.detail = detail;
    servers.push(server);
  }
  return servers;
}

/** What picking this server does. */
export function actionFor(server: McpServer): McpAction {
  return SHOWN[server.status].action;
}

/** The picker: a server a row, `busy` (being acted on) marked as such. */
export function mcpChoice(servers: McpServer[], busy?: string, note?: string): Omit<Choice, "id"> {
  const options = servers.map((server) => {
    const shown = SHOWN[server.status];
    const name = `${server.name} · ${server.name === busy ? "…" : shown.label}`;
    const parts = [server.name === busy ? "working on it" : `pick to ${shown.action}`];
    if (server.detail !== undefined) parts.push(server.detail);
    return { id: server.name, name, description: parts.join(" · ") };
  });
  let title = "MCP servers (pick one to enable, disable or reconnect it)";
  if (note !== undefined) title += ` · ${note}`;
  return { type: "choice", title, options, stays: true };
}

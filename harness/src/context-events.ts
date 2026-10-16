import type { ContextEvent } from "./agent.ts";

function escapeAttr(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function attrs(pairs: [string, string][]): string {
  return pairs.map(([key, value]) => `${key}="${escapeAttr(value)}"`).join(" ");
}

/**
 * The first time: everything (with what the app is for). Back with the same version: just that.
 * A new version the session knew an older one of: what changed.
 */
function connectedAttrs(
  event: Extract<ContextEvent, { type: "app_connected" }>,
): [string, string][] {
  const v: [string, string] = ["v", String(event.v)];
  if (event.reconnect === true) return [["reconnected", event.app], v];
  if (event.previous !== undefined) {
    const pairs: [string, string][] = [["updated", event.app], v];
    if (event.added?.length) pairs.push(["added", event.added.join(", ")]);
    if (event.removed?.length) pairs.push(["removed", event.removed.join(", ")]);
    return pairs;
  }
  const pairs: [string, string][] = [
    ["connected", event.app],
    ["kind", event.kind],
    v,
    ["tools", event.tools.join(", ")],
  ];
  if (event.description !== undefined) pairs.push(["about", event.description]);
  return pairs;
}

/**
 * Prepends connector events to the user's text (design §6). Connect/disconnect lines and guest
 * turns keep their order; only the last focus event is sent, after them. `cwd` (the session's working directory)
 * comes first, on a session's first prompt.
 */
export function formatPrompt(text: string, pendingEvents: ContextEvent[], cwd?: string): string {
  const lines: string[] = [];
  if (cwd !== undefined) lines.push(`<session ${attrs([["cwd", cwd]])}/>`);
  let focus: string | undefined;
  for (const event of pendingEvents) {
    if (event.type === "app_connected") {
      lines.push(`<app ${attrs(connectedAttrs(event))}/>`);
    } else if (event.type === "app_disconnected") {
      lines.push(`<app ${attrs([["disconnected", event.app]])}/>`);
    } else if (event.type === "guest_turn") {
      const pairs: [string, string][] = [
        ["agent", event.agent],
        ["task", event.task],
        ["stop", event.stopReason],
      ];
      lines.push(`<guest ${attrs(pairs)}>`, event.result, "</guest>");
    } else {
      const pairs: [string, string][] = [
        ["app", event.app],
        ...Object.entries(event.context ?? {}),
      ];
      focus = `<focus ${attrs(pairs)}/>`;
    }
  }
  if (focus) lines.push(focus);
  lines.push(text);
  return lines.join("\n");
}

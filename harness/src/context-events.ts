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
 * Prepends connector events to the user's text (design §6). Connect/disconnect lines keep their
 * order; only the last focus event is sent, after them.
 */
export function formatPrompt(text: string, pendingEvents: ContextEvent[]): string {
  const lines: string[] = [];
  let focus: string | undefined;
  for (const event of pendingEvents) {
    if (event.type === "app_connected") {
      const pairs: [string, string][] = [
        ["connected", event.app],
        ["kind", event.kind],
        ["v", String(event.v)],
        ["tools", event.tools.join(", ")],
      ];
      lines.push(`<app ${attrs(pairs)}/>`);
    } else if (event.type === "app_disconnected") {
      lines.push(`<app ${attrs([["disconnected", event.app]])}/>`);
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

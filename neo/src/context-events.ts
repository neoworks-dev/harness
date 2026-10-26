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
    if (event.components !== undefined) pairs.push(["components", event.components.join(", ")]);
    return pairs;
  }
  const pairs: [string, string][] = [
    ["connected", event.app],
    ["kind", event.kind],
    v,
    ["tools", event.tools.join(", ")],
  ];
  if (event.components?.length) pairs.push(["components", event.components.join(", ")]);
  if (event.description !== undefined) pairs.push(["about", event.description]);
  return pairs;
}

/** What the agent is asked when the user turns "Show, don't tell." on; the UI library follows. */
export const SHOW_DONT_TELL_ON = `The user turned on "Show, don't tell.": answer with UI more than text.
Whenever an answer has a shape (options to pick from, steps, a comparison, numbers, a status, a
list of files or results, something to fill in or confirm), write it as a \`\`\`neoui block
instead of describing it, and keep the prose around it to a sentence or two. A short answer or a
conversation is still just text. The UI library follows: there is no need to call ui_library.`;

/** What the agent is told when the user turns "Show, don't tell." off again. */
export const SHOW_DONT_TELL_OFF = `The user turned off "Show, don't tell.": answer in text again,
with UI only where it clearly helps.`;

/**
 * Prepends connector events to the user's text (design §6). Connect/disconnect lines, guest
 * turns, reverted edits and the user's notes keep their order; only the last focus event is
 * sent, after them.
 * `cwd` (the session's working directory) comes first, on a session's first prompt; then the
 * `summary` of turns another harness ran, if any.
 */
export function formatPrompt(
  text: string,
  pendingEvents: ContextEvent[],
  cwd?: string,
  summary?: string,
): string {
  const lines: string[] = [];
  if (cwd !== undefined) lines.push(`<session ${attrs([["cwd", cwd]])}/>`);
  if (summary !== undefined) lines.push(summary);
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
    } else if (event.type === "edits_reverted") {
      // Nothing was put back (everything changed again since): nothing for the agent to know.
      if (event.paths.length === 0) continue;
      const pairs: [string, string][] = [["paths", event.paths.join(", ")]];
      if (event.skipped?.length) pairs.push(["skipped", event.skipped.join(", ")]);
      lines.push(`<reverted ${attrs(pairs)}/>`);
    } else if (event.type === "user_note") {
      const open =
        event.about === undefined ? "<note>" : `<note ${attrs([["about", event.about]])}>`;
      lines.push(open, event.note, "</note>");
    } else if (event.type === "show_dont_tell") {
      lines.push(`<show-dont-tell ${attrs([["on", String(event.on)]])}>`);
      if (event.on) {
        lines.push(SHOW_DONT_TELL_ON);
        if (event.library !== undefined) lines.push("", event.library);
      } else {
        lines.push(SHOW_DONT_TELL_OFF);
      }
      lines.push("</show-dont-tell>");
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

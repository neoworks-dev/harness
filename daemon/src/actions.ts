// ui.action: a button in a view was pressed. An action naming an open app's tool (app.tool) runs
// that tool directly, and its result shows in the chat like any tool call (rendered with the
// app's template). Otherwise the agent gets it as input, so it can act on it or explain.
import type { Component, UiAction } from "@suite/protocol/types";
import { BadRequestError } from "./focus.ts";
import type { Gateway } from "./gateway/gateway.ts";
import type { ConnectorRegistry } from "./gateway/registry.ts";
import type { GuestManager } from "./guests/guests.ts";
import type { SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";
import type { ViewManager } from "./view/views.ts";

interface Parts {
  store: SessionStore;
  sessions: SessionManager;
  connectors: ConnectorRegistry;
  gateway: Gateway;
  views: ViewManager;
  guests?: GuestManager;
}

function findLabel(component: Component, action: string): string | undefined {
  if (component.type === "button") return component.action === action ? component.label : undefined;
  if ("children" in component) {
    for (const child of component.children) {
      const label = findLabel(child, action);
      if (label !== undefined) return label;
    }
  }
  return undefined;
}

export class ActionRunner {
  constructor(private readonly parts: Parts) {}

  async run({ session, ref, action, args }: UiAction, from: string): Promise<void> {
    if (action.startsWith("copy:")) {
      throw new BadRequestError("copy: actions are handled by the client");
    }
    if (action === "suite.permit") {
      // A guest's permission request (permission_request): args {id, option}.
      const fields: Record<string, unknown> = Object.assign({}, args);
      const { id, option } = fields;
      if (typeof id !== "string" || typeof option !== "string" || !this.parts.guests) {
        throw new BadRequestError("suite.permit needs args {id, option}");
      }
      this.parts.guests.permit(session, id, option, from);
      return;
    }
    const { store, connectors, gateway, sessions, views } = this.parts;
    const dot = action.indexOf(".");
    const app = action.slice(0, dot);
    const tool = action.slice(dot + 1);
    const known = dot > 0 && connectors.connector(app)?.tools.some((t) => t.name === tool) === true;

    if (known && connectors.isOpen(app)) {
      const toolCallId = `ui-${crypto.randomUUID()}`;
      const callArgs: Record<string, unknown> = {};
      if (typeof args === "object" && args !== null) Object.assign(callArgs, args);
      store.append(session, {
        type: "tool_call",
        toolCallId,
        tool: action,
        title: action,
        kind: "other",
        status: "in_progress",
        rawInput: callArgs,
      });
      const result = await gateway.call("tools_call", { name: action, args: callArgs });
      const lines: string[] = [];
      for (const c of result.content) if (c.type === "text") lines.push(c.text);
      const text = lines.join("\n");
      const update: Parameters<SessionStore["append"]>[1] = {
        type: "tool_call_update",
        toolCallId,
        status: result.isError ? "failed" : "completed",
        content: [{ type: "text", text }],
      };
      if (result.structuredContent !== undefined) update.rawOutput = result.structuredContent;
      store.append(session, update);
      return;
    }

    const item = views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === ref);
    const label = item ? findLabel(item.component, action) : undefined;
    const pressed = label === undefined ? `a button` : `"${label}"`;
    const why = known ? `${app} isn't open` : `no open app handles it`;
    sessions.input(
      session,
      from,
      `[ui action] The user pressed ${pressed} (${action} ${JSON.stringify(args ?? {})}) in an earlier result; ${why}.`,
    );
  }
}

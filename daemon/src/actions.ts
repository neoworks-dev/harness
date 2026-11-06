// ui.action: a button in a view was pressed. An action naming an open app's tool (app.tool) runs
// that tool directly, and its result shows in the chat like any tool call (rendered with the
// app's template). neoui.input and neoui.press belong to a neoui doc (docs/neoui.md).
// Otherwise the agent gets it as input, so it can act on it or explain.
import type { AssistantMessage, CallTool } from "@neoworks/neoui";
import type { Component, UiAction } from "@neoworks-dev/neo-protocol/types";
import { BadRequestError } from "./focus.ts";
import type { Gateway } from "./gateway/gateway.ts";
import type { ConnectorRegistry } from "./gateway/registry.ts";
import type { Asker } from "./asker.ts";
import type { SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";
import type { ViewManager } from "./view/views.ts";
import { parseTodoArgs, type Todos } from "./todos.ts";

export interface Parts {
  store: SessionStore;
  sessions: SessionManager;
  connectors: ConnectorRegistry;
  gateway: Gateway;
  views: ViewManager;
  asker?: Asker;
  /** Stops one running subagent of the session (neo.cancel_agent {id}). */
  cancelAgent?: (session: string, id: string) => boolean;
  /** The shared todo list the user changes from an app (neo.todos {action, text?, id?}). */
  todos?: Todos;
}

/**
 * Runs an app's tool for the user and records it in the session like a tool call of the agent's,
 * so its result shows in the chat (drawn with the app's template).
 */
async function runAppTool(
  parts: Pick<Parts, "store" | "gateway">,
  session: string,
  name: string,
  args: Record<string, unknown>,
): Promise<{ isError: boolean; text: string; structured?: unknown }> {
  const { store, gateway } = parts;
  const toolCallId = `ui-${crypto.randomUUID()}`;
  store.append(session, {
    type: "tool_call",
    toolCallId,
    tool: name,
    title: name,
    kind: "other",
    status: "in_progress",
    rawInput: args,
  });
  const result = await gateway.call("tools_call", { name, args });
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
  const out: { isError: boolean; text: string; structured?: unknown } = {
    isError: result.isError === true,
    text,
  };
  if (result.structuredContent !== undefined) out.structured = result.structuredContent;
  return out;
}

/** What a tool returned as data: its structured result, else its text, as JSON when it is. */
function toolData(result: { text: string; structured?: unknown }): unknown {
  if (result.structured !== undefined) return result.structured;
  try {
    return JSON.parse(result.text);
  } catch {
    return result.text;
  }
}

/**
 * The tools a neoui program's Query and Mutation call, for a session. A mutation is something
 * the user did, so it is recorded like a button that runs a tool; a query is a read and is not.
 */
export function neoUiTools(parts: Pick<Parts, "store" | "gateway">, session: string): CallTool {
  return async (name, args, kind) => {
    if (kind === "mutation") {
      const result = await runAppTool(parts, session, name, args);
      if (result.isError) throw new Error(result.text);
      return toolData(result);
    }
    const result = await parts.gateway.call("tools_call", { name, args });
    const lines: string[] = [];
    for (const c of result.content) if (c.type === "text") lines.push(c.text);
    const text = lines.join("\n");
    if (result.isError) throw new Error(text);
    return toolData({ text, structured: result.structuredContent });
  };
}

/**
 * What a button of a neoui doc sends: for the agent, what was pressed with the form's values and
 * the context; for the chat, the message as if the user had written it, with what they picked in
 * a paragraph under it, by the doc's labels.
 */
function neoUiInput(sent: AssistantMessage): { text: string; shown: string } {
  const pressed = `[ui action] The user pressed "${sent.label}"`;
  // A button with no ui-ask sends its label: don't say it twice.
  const lines = [sent.message === sent.label ? `${pressed}.` : `${pressed}: ${sent.message}`];
  if (sent.values) {
    const where = sent.form === undefined ? "state" : `form ${sent.form}`;
    lines.push(`${where}: ${JSON.stringify(sent.values)}`);
  }
  if (sent.context !== undefined) lines.push(`context: ${JSON.stringify(sent.context)}`);
  const picks = (sent.picked ?? []).map(({ label, value }) => `${label}: ${value}`);
  const shown = picks.length === 0 ? sent.message : `${sent.message}\n\n${picks.join(" · ")}`;
  return { text: lines.join("\n"), shown };
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
    if (action === "neo.permit" || action === "neo.choose") {
      // Answers a permission_request or a choice: args {id, option, note?, stay?, value?}; stay
      // keeps a choice that stays open (it acts on the option and shows again), value is the
      // text an option with an input asked for.
      const fields: Record<string, unknown> = Object.assign({}, args);
      const { id, option, note, stay, value } = fields;
      if (typeof id !== "string" || typeof option !== "string" || !this.parts.asker) {
        throw new BadRequestError(`${action} needs args {id, option}`);
      }
      if (note !== undefined && typeof note !== "string") {
        throw new BadRequestError(`${action}: note must be a string`);
      }
      if (value !== undefined && typeof value !== "string") {
        throw new BadRequestError(`${action}: value must be a string`);
      }
      const kind = action === "neo.permit" ? "permission" : "choice";
      // An empty note is no note.
      const trimmed = note?.trim();
      const added = trimmed === "" ? undefined : trimmed;
      this.parts.asker.answer(session, kind, id, option, from, added, stay === true, value);
      return;
    }
    if (action === "neo.close") {
      // Closes a choice that stays open (`/settings`): args {id}.
      const fields: Record<string, unknown> = Object.assign({}, args);
      const { id } = fields;
      if (typeof id !== "string" || !this.parts.asker) {
        throw new BadRequestError("neo.close needs args {id}");
      }
      this.parts.asker.close(session, id);
      return;
    }
    if (action === "neoui.input" || action === "neoui.press") {
      await this.neoUi(session, ref, action, args, from);
      return;
    }
    if (action === "neo.cancel_agent") {
      const fields: Record<string, unknown> = Object.assign({}, args);
      const { id } = fields;
      if (typeof id !== "string" || !this.parts.cancelAgent) {
        throw new BadRequestError("neo.cancel_agent needs args {id}");
      }
      if (!this.parts.cancelAgent(session, id)) {
        throw new BadRequestError(`${id} isn't a running subagent of this session`);
      }
      return;
    }
    if (action === "neo.background") {
      // Sends a running command of the session's agent to the background: args {toolCallId}.
      const fields: Record<string, unknown> = Object.assign({}, args);
      const { toolCallId } = fields;
      if (typeof toolCallId !== "string") {
        throw new BadRequestError("neo.background needs args {toolCallId}");
      }
      if (!(await this.parts.sessions.background(session, toolCallId))) {
        throw new BadRequestError("this command can't be sent to the background");
      }
      return;
    }
    if (action === "neo.todos") {
      const fields: Record<string, unknown> = Object.assign({}, args);
      const change = parseTodoArgs(fields);
      if (!this.parts.todos || change === undefined || typeof change === "string") {
        throw new BadRequestError(
          typeof change === "string" ? change : "neo.todos needs args {action, text?, id?}",
        );
      }
      const result = this.parts.todos.change(session, change, "user");
      if (typeof result === "string") throw new BadRequestError(`todos: ${result}`);
      return;
    }
    const { connectors, sessions, views } = this.parts;
    const dot = action.indexOf(".");
    const app = action.slice(0, dot);
    const tool = action.slice(dot + 1);
    const known = dot > 0 && connectors.connector(app)?.tools.some((t) => t.name === tool) === true;

    if (known && connectors.isOpen(app)) {
      const callArgs: Record<string, unknown> = {};
      if (typeof args === "object" && args !== null) Object.assign(callArgs, args);
      await runAppTool(this.parts, session, action, callArgs);
      return;
    }

    const item = views.get(session).item(ref);
    const label = item ? findLabel(item.component, action) : undefined;
    const pressed = label === undefined ? `a button` : `"${label}"`;
    const why = known ? `${app} isn't open` : `no open app handles it`;
    sessions.input(
      session,
      from,
      `[ui action] The user pressed ${pressed} (${action} ${JSON.stringify(args ?? {})}) in an earlier result; ${why}.`,
    );
  }

  /**
   * neoui.input {doc, key, value} and neoui.press {doc, key}. Actions on a doc that is still
   * streaming, or on a node it no longer has, change nothing.
   */
  private async neoUi(
    session: string,
    ref: string,
    action: string,
    args: unknown,
    from: string,
  ): Promise<void> {
    const fields: Record<string, unknown> = Object.assign({}, args);
    const { doc, key, value } = fields;
    if (typeof doc !== "string" || typeof key !== "string") {
      throw new BadRequestError(`${action} needs args {doc, key}`);
    }
    const found = this.parts.views.neoUi(session, ref, doc);
    if (!found) throw new BadRequestError(`${ref} has no neoui doc ${doc}`);
    const { program, view } = found;
    if (action === "neoui.input") {
      if (program.input(key, value)) view.changed(ref);
      return;
    }
    const result = await program.press(key);
    view.changed(ref);
    for (const sent of result.assistant) {
      const { text, shown } = neoUiInput(sent);
      this.parts.sessions.input(session, from, text, undefined, shown);
    }
  }
}

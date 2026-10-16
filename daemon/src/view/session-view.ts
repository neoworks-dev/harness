// A session's chat as drawn: session events → view items, and each new event → the ops that
// update every pane. Streaming text is merged here (re-parsed, only changed tail blocks sent);
// tool results are filled into their app's template, or a generic card when there is none.
import type {
  Block,
  Component,
  ItemState,
  ListItem,
  SessionEvent,
  SessionEventRecord,
  Span,
  ToolCallContent,
  ToolCallStatus,
  ViewItem,
  ViewOp,
  ViewSnapshot,
} from "@suite/protocol/types";
import type { Logger } from "../log.ts";
import { diffLines } from "./diff.ts";
import { parseMarkdown } from "./markdown.ts";
import { resolveTemplate, TemplateError } from "./template.ts";

/** Finds the template registered for a tool (app.tool), if any. */
export type TemplateLookup = (tool: string) => unknown;

interface Stream {
  ref: string;
  raw: string;
  blocks: Block[];
}

interface Call {
  ref: string;
  /** Set for a guest's calls: their rawOutput is the guest's, not the connector's. */
  agent?: string;
  tool?: string;
  title: string;
  status: ToolCallStatus;
  rawInput?: unknown;
  content?: ToolCallContent[];
  rawOutput?: unknown;
}

type Plan = Extract<SessionEvent, { type: "plan" }>;
type PermissionRequest = Extract<SessionEvent, { type: "permission_request" }>;
type Choice = Extract<SessionEvent, { type: "choice" }>;

const RESULT_REF = /\[result:([0-9a-f]+)\]/;

const STATES: Record<ToolCallStatus, ItemState> = {
  pending: "pending",
  in_progress: "running",
  completed: "done",
  failed: "failed",
};

function text(
  blocks: Block[],
  extra: Partial<Extract<Component, { type: "text" }>> = {},
): Component {
  return { type: "text", ...extra, blocks };
}

function plain(value: string): Block[] {
  return value
    .split(/\n{2,}/)
    .filter((p) => p !== "")
    .map((p) => ({ type: "paragraph", spans: [{ text: p }] }));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function connectedNotice(event: Extract<SessionEvent, { type: "app_connected" }>): string {
  if (event.reconnect === true) return `${event.app} is back (v${event.v})`;
  if (event.previous === undefined) {
    return `${event.app} connected (v${event.v}): ${event.tools.join(", ")}`;
  }
  const changes: string[] = [];
  if (event.added?.length) changes.push(`+ ${event.added.join(", ")}`);
  if (event.removed?.length) changes.push(`− ${event.removed.join(", ")}`);
  const what = changes.length > 0 ? changes.join("; ") : "schemas changed";
  return `${event.app} updated (v${event.v}): ${what}`;
}

/** The first index where two block lists differ. */
function firstChange(before: Block[], after: Block[]): number {
  let i = 0;
  while (
    i < before.length &&
    i < after.length &&
    JSON.stringify(before[i]) === JSON.stringify(after[i])
  )
    i++;
  return i;
}

export class SessionView {
  private readonly items: ViewItem[] = [];
  private readonly index = new Map<string, number>();
  private readonly streams = new Map<string, Stream>();
  private readonly calls = new Map<string, Call>();
  /** Connector results handed to guests as [result:ref]. */
  private readonly results = new Map<string, { tool: string; rawOutput: unknown }>();
  private readonly permissions = new Map<string, PermissionRequest>();
  private readonly choices = new Map<string, Choice>();
  private seq = 0;

  constructor(
    private readonly session: string,
    private readonly templates: TemplateLookup,
    private readonly log: Logger,
  ) {}

  snapshot(): ViewSnapshot {
    return { session: this.session, seq: this.seq, items: structuredClone(this.items) };
  }

  /** Applies one event and returns the ops that bring a pane from the previous state to this one. */
  apply(record: SessionEventRecord): ViewOp[] {
    this.seq = record.seq;
    const ops = this.opsFor(record.event, record.seq);
    for (const op of ops) this.applyOp(op);
    return ops;
  }

  private opsFor(event: SessionEvent, seq: number): ViewOp[] {
    switch (event.type) {
      case "user_message": {
        const children: Component[] = [text(plain(event.text))];
        for (const src of event.images ?? []) children.push({ type: "image", src });
        const component: Component =
          children.length === 1 && children[0]
            ? children[0]
            : { type: "stack", gap: "s", children };
        return [{ op: "set", item: { ref: `u${seq}`, kind: "user", label: event.app, component } }];
      }
      case "agent_message_chunk":
        return this.stream(`m-${event.messageId}`, "agent", event.text, event.agent);
      case "agent_thought_chunk":
        return this.stream(`t-${event.messageId}`, "thought", event.text, event.agent);
      case "tool_call": {
        const call: Call = {
          ref: `c-${event.toolCallId}`,
          title: event.agent === undefined ? event.title : `${event.agent}: ${event.title}`,
          status: event.status,
          rawInput: event.rawInput,
        };
        if (event.tool !== undefined) call.tool = event.tool;
        if (event.agent !== undefined) call.agent = event.agent;
        this.calls.set(event.toolCallId, call);
        return [{ op: "set", item: this.toolItem(call) }];
      }
      case "tool_call_update": {
        const call = this.calls.get(event.toolCallId);
        if (!call) return [];
        if (event.title !== undefined) {
          call.title = call.agent === undefined ? event.title : `${call.agent}: ${event.title}`;
        }
        if (event.tool !== undefined) call.tool = event.tool;
        if (event.rawInput !== undefined) call.rawInput = event.rawInput;
        if (event.status !== undefined) call.status = event.status;
        if (event.content !== undefined) call.content = event.content;
        if (event.rawOutput !== undefined) call.rawOutput = event.rawOutput;
        return [{ op: "set", item: this.toolItem(call) }];
      }
      case "tool_result":
        this.results.set(event.ref, { tool: event.tool, rawOutput: event.rawOutput });
        return [];
      case "permission_request":
        this.permissions.set(event.id, event);
        return [{ op: "set", item: this.permissionItem(event) }];
      case "permission_resolved": {
        const request = this.permissions.get(event.id);
        if (!request) return [];
        return [{ op: "set", item: this.permissionItem(request, event.option ?? null) }];
      }
      case "choice":
        this.choices.set(event.id, event);
        return [{ op: "set", item: this.choiceItem(event) }];
      case "choice_made": {
        const choice = this.choices.get(event.id);
        if (!choice) return [];
        return [{ op: "set", item: this.choiceItem(choice, event.option ?? null) }];
      }
      case "plan":
        return this.planOps(event);
      case "setting_changed": {
        const what =
          event.option === "approvals" ? "approvals" : `${event.harness} ${event.option}`;
        return [this.notice(`n${seq}`, `${what}: ${event.value}`)];
      }
      case "turn_ended": {
        if (event.stopReason === "end_turn") return [];
        const what = event.error ?? `stopped: ${event.stopReason.replace("_", " ")}`;
        const tone = event.stopReason === "cancelled" ? "muted" : "danger";
        return [this.notice(`e${seq}`, what, tone)];
      }
      case "app_connected":
        return [this.notice(`n${seq}`, connectedNotice(event))];
      case "app_disconnected":
        return [this.notice(`n${seq}`, `${event.app} disconnected`)];
      case "focus_changed": {
        const context = Object.entries(event.context ?? {}).map(([k, v]) => ` ${k}=${v}`);
        return [this.notice(`n${seq}`, `now in ${event.app}${context.join("")}`)];
      }
      default:
        return [];
    }
  }

  private notice(ref: string, message: string, tone: "muted" | "danger" = "muted"): ViewOp {
    return { op: "set", item: { ref, kind: "notice", component: text(plain(message), { tone }) } };
  }

  /**
   * An agent's todo list: one item per agent, replaced with each new plan. An empty plan removes
   * the item.
   */
  private planOps(plan: Plan): ViewOp[] {
    const ref = plan.agent === undefined ? "plan" : `plan-${plan.agent}`;
    if (plan.entries.length === 0) {
      if (!this.index.has(ref)) return [];
      return [{ op: "remove", ref }];
    }
    const items: ListItem[] = plan.entries.map((entry) => {
      const inProgress = entry.status === "in_progress";
      const span: Span = inProgress
        ? { text: `◐ ${entry.content}`, bold: true }
        : { text: entry.content };
      return {
        checked: entry.status === "completed",
        blocks: [{ type: "paragraph", spans: [span] }],
      };
    });
    let state: ItemState = "pending";
    if (plan.entries.every((entry) => entry.status === "completed")) state = "done";
    else if (plan.entries.some((entry) => entry.status === "in_progress")) state = "running";
    const item: ViewItem = {
      ref,
      kind: "tool",
      label: plan.agent === undefined ? "Plan" : `${plan.agent}: Plan`,
      state,
      component: text([{ type: "list", ordered: false, items }]),
    };
    return [{ op: "set", item }];
  }

  /**
   * A guest asking before it runs a tool: one button per option until answered (answer: the
   * picked option id, or null when nobody answered before the turn ended).
   */
  private permissionItem(request: PermissionRequest, answer?: string | null): ViewItem {
    const label = `${request.agent} asks: ${request.title}`;
    if (answer === undefined) {
      const buttons: Component[] = request.options.map((option, i) => ({
        type: "button",
        label: option.name,
        action: "suite.permit",
        args: { id: request.id, option: option.id },
        key: String(i + 1),
      }));
      const component: Component = { type: "row", gap: "s", children: buttons };
      return { ref: `p-${request.id}`, kind: "tool", label, state: "pending", component };
    }
    const option = request.options.find((o) => o.id === answer);
    const allowed = option?.kind.startsWith("allow") === true;
    const what = option ? option.name : "No answer: the turn ended.";
    return {
      ref: `p-${request.id}`,
      kind: "tool",
      label,
      state: allowed ? "done" : "failed",
      component: text(plain(what), { tone: "muted" }),
    };
  }

  /** A choice: one button per option (with its description, if any) until one is picked. */
  private choiceItem(choice: Choice, answer?: string | null): ViewItem {
    const ref = `q-${choice.id}`;
    if (answer === undefined) {
      const rows: Component[] = choice.options.map((option, i) => {
        const button: Component = {
          type: "button",
          label: option.name,
          action: "suite.choose",
          args: { id: choice.id, option: option.id },
        };
        if (i < 9) button.key = String(i + 1);
        if (option.description === undefined) return button;
        return {
          type: "row",
          gap: "s",
          children: [button, text(plain(option.description), { tone: "muted" })],
        };
      });
      const component: Component = { type: "stack", gap: "s", children: rows };
      return { ref, kind: "tool", label: choice.title, state: "pending", component };
    }
    const option = choice.options.find((o) => o.id === answer);
    return {
      ref,
      kind: "tool",
      label: choice.title,
      state: option ? "done" : "failed",
      component: text(plain(option ? option.name : "Not answered."), { tone: "muted" }),
    };
  }

  /** Streaming text: re-parse the whole message, send only the blocks that changed. */
  private stream(
    ref: string,
    kind: "agent" | "thought",
    chunk: string,
    agent: string | undefined,
  ): ViewOp[] {
    const existing = this.streams.get(ref);
    const stream = existing ?? { ref, raw: "", blocks: [] };
    stream.raw += chunk;
    const blocks = parseMarkdown(stream.raw);
    const from = firstChange(stream.blocks, blocks);
    stream.blocks = blocks;
    this.streams.set(ref, stream);
    if (!existing) {
      const component = kind === "thought" ? text(blocks, { tone: "muted" }) : text(blocks);
      const item: ViewItem = { ref, kind, component };
      if (agent !== undefined) item.label = agent;
      return [{ op: "set", item }];
    }
    return [{ op: "blocks", ref, path: [], from, blocks: blocks.slice(from) }];
  }

  private toolItem(call: Call): ViewItem {
    return {
      ref: call.ref,
      kind: "tool",
      label: call.title,
      state: STATES[call.status],
      component: this.resultView(call),
    };
  }

  private resultView(call: Call): Component {
    const result = this.connectorResult(call);
    const template = result && this.templates(result.tool);
    if (result && template !== undefined && call.status === "completed") {
      try {
        return resolveTemplate(template, result.rawOutput);
      } catch (err) {
        const why = err instanceof TemplateError ? err.message : String(err);
        this.log.warn(`template for ${result.tool} failed: ${why}`);
      }
    }
    return this.fallback(call);
  }

  /** The connector result a call returned: its own rawOutput, or a guest's [result:ref]. */
  private connectorResult(call: Call): { tool: string; rawOutput: unknown } | undefined {
    if (call.agent === undefined) {
      if (call.tool === undefined || !call.tool.includes(".") || call.rawOutput === undefined) {
        return undefined;
      }
      return { tool: call.tool, rawOutput: call.rawOutput };
    }
    for (const content of call.content ?? []) {
      const ref = content.type === "text" ? RESULT_REF.exec(content.text)?.[1] : undefined;
      const result = ref === undefined ? undefined : this.results.get(ref);
      if (result) return result;
    }
    return undefined;
  }

  /** No template: what ACP's base data says (text output, diffs), or the structured result as kv. */
  private fallback(call: Call): Component {
    const children: Component[] = [];
    for (const content of call.content ?? []) {
      if (content.type === "diff") {
        children.push({
          type: "diff",
          path: content.path,
          lines: diffLines(content.oldText, content.newText),
        });
      } else if (content.text.trim() !== "") {
        children.push({ type: "code", code: content.text.replace(/\n+$/, "") });
      }
    }
    if (children.length === 0 && isRecord(call.rawOutput)) {
      const items = Object.entries(call.rawOutput)
        .filter(([, v]) => typeof v !== "object" || v === null)
        .map(([key, value]) => ({ key, value: String(value) }));
      if (items.length > 0) children.push({ type: "kv", items });
    }
    if (children.length === 1 && children[0]) return children[0];
    return { type: "stack", gap: "s", children };
  }

  private applyOp(op: ViewOp): void {
    switch (op.op) {
      case "set": {
        const at = this.index.get(op.item.ref);
        if (at === undefined) {
          this.index.set(op.item.ref, this.items.length);
          this.items.push(op.item);
        } else {
          this.items[at] = op.item;
        }
        break;
      }
      case "blocks": {
        const at = this.index.get(op.ref);
        const item = at === undefined ? undefined : this.items[at];
        // Only root text components stream today (path []).
        if (item?.component.type === "text") {
          item.component.blocks = [...item.component.blocks.slice(0, op.from), ...op.blocks];
        }
        break;
      }
      case "remove": {
        const at = this.index.get(op.ref);
        if (at === undefined) break;
        this.items.splice(at, 1);
        this.index.clear();
        this.items.forEach((item, i) => this.index.set(item.ref, i));
        break;
      }
    }
  }
}

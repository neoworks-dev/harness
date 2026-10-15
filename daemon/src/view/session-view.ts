// A session's chat as drawn: session events → view items, and each new event → the ops that
// update every pane. Streaming text is merged here (re-parsed, only changed tail blocks sent);
// tool results are filled into their app's template, or a generic card when there is none.
import type {
  Block,
  Component,
  ItemState,
  SessionEvent,
  SessionEventRecord,
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
  tool?: string;
  title: string;
  status: ToolCallStatus;
  rawInput?: unknown;
  content?: ToolCallContent[];
  rawOutput?: unknown;
}

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
        return this.stream(`m-${event.messageId}`, "agent", event.text);
      case "agent_thought_chunk":
        return this.stream(`t-${event.messageId}`, "thought", event.text);
      case "tool_call": {
        const call: Call = {
          ref: `c-${event.toolCallId}`,
          title: event.title,
          status: event.status,
          rawInput: event.rawInput,
        };
        if (event.tool !== undefined) call.tool = event.tool;
        this.calls.set(event.toolCallId, call);
        return [{ op: "set", item: this.toolItem(call) }];
      }
      case "tool_call_update": {
        const call = this.calls.get(event.toolCallId);
        if (!call) return [];
        if (event.status !== undefined) call.status = event.status;
        if (event.content !== undefined) call.content = event.content;
        if (event.rawOutput !== undefined) call.rawOutput = event.rawOutput;
        return [{ op: "set", item: this.toolItem(call) }];
      }
      case "turn_ended": {
        if (event.stopReason === "end_turn") return [];
        const what = event.error ?? `stopped: ${event.stopReason.replace("_", " ")}`;
        const tone = event.stopReason === "cancelled" ? "muted" : "danger";
        return [this.notice(`e${seq}`, what, tone)];
      }
      case "app_connected":
        return [
          this.notice(`n${seq}`, `${event.app} connected (v${event.v}): ${event.tools.join(", ")}`),
        ];
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

  /** Streaming text: re-parse the whole message, send only the blocks that changed. */
  private stream(ref: string, kind: "agent" | "thought", chunk: string): ViewOp[] {
    const existing = this.streams.get(ref);
    const stream = existing ?? { ref, raw: "", blocks: [] };
    stream.raw += chunk;
    const blocks = parseMarkdown(stream.raw);
    const from = firstChange(stream.blocks, blocks);
    stream.blocks = blocks;
    this.streams.set(ref, stream);
    if (!existing) {
      const component = kind === "thought" ? text(blocks, { tone: "muted" }) : text(blocks);
      return [{ op: "set", item: { ref, kind, component } }];
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
    const template =
      call.tool !== undefined && call.tool.includes(".") ? this.templates(call.tool) : undefined;
    if (template !== undefined && call.status === "completed" && call.rawOutput !== undefined) {
      try {
        return resolveTemplate(template, call.rawOutput);
      } catch (err) {
        const why = err instanceof TemplateError ? err.message : String(err);
        this.log.warn(`template for ${call.tool ?? "?"} failed: ${why}`);
      }
    }
    return this.fallback(call);
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

// A session's chat as drawn: session events → view items, and each new event → the ops that
// update every pane. Streaming text is merged here (re-parsed, only changed tail blocks sent);
// tool results are filled into their app's template, or a generic card when there is none.
import type { AppLibrary, CallTool, Program } from "@neoworks/neoui";
import type {
  Block,
  Component,
  ItemState,
  ListItem,
  SessionEvent,
  SessionEventRecord,
  Span,
  SurfaceOpened,
  ToolCallContent,
  ToolCallStatus,
  ToolKind,
  ViewItem,
  ViewOp,
  ViewSnapshot,
} from "@neoworks-dev/neo-protocol/types";
import type { Logger } from "../log.ts";
import { MAIN, MESSAGES_APP } from "../messages.ts";
import { diffLines } from "./diff.ts";
import { GrowingMarkdown, parseMarkdown } from "./markdown.ts";
import { isNeoUiTemplate, NeoUiDocs, templateData } from "./neoui.ts";
import { recognize } from "./recognize.ts";
import { resolveTemplate, TemplateError } from "./template.ts";

/** Finds the template registered for a tool (app.tool), if any. */
export type TemplateLookup = (tool: string) => unknown;

/** The ref of the surface `name`: what patches and ui.action address it by. */
export function surfaceRef(name: string): string {
  return `s-${name}`;
}

interface Stream {
  ref: string;
  kind: "agent" | "thought";
  raw: string;
  blocks: Block[];
  /** The message is complete: a neoui fence still open is a finished program. */
  ended: boolean;
  markdown: GrowingMarkdown;
}

interface Call {
  ref: string;
  /** Set for a guest's calls: their rawOutput is the guest's, not the connector's. */
  agent?: string;
  tool?: string;
  title: string;
  kind: ToolKind;
  status: ToolCallStatus;
  rawInput?: unknown;
  content?: ToolCallContent[];
  rawOutput?: unknown;
  /** A message it sent to another agent (see sentMessage): that agent's answer, once it came. */
  reply?: string;
}

/**
 * A call that sends another agent a message: Claude Code's SendMessage `{to, message}` (to its
 * subagent's handle), neo's send_message `{to, text}` (as any harness names it).
 */
function sentMessage(call: Call): { to: string; text: string } | undefined {
  if (!isRecord(call.rawInput)) return undefined;
  const input = isRecord(call.rawInput.arguments) ? call.rawInput.arguments : call.rawInput;
  const title = call.agent === undefined ? call.title : call.title.slice(call.agent.length + 2);
  const { to, message, text } = input;
  if (typeof to !== "string") return undefined;
  if (title === "SendMessage" && typeof message === "string") return { to, text: message };
  if (/^message /.test(title) && typeof text === "string") return { to, text };
  return undefined;
}

/** Why a sent message didn't arrive: Claude Code answers `{"success": false, "message": …}`. */
function sendFailure(call: Call): string | undefined {
  const said = (call.content ?? [])
    .filter((c) => c.type === "text")
    .map((c) => c.text)
    .join("");
  const raw = said === "" ? call.rawOutput : said;
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    try {
      parsed = JSON.parse(raw);
    } catch {
      return call.status === "failed" ? raw : undefined;
    }
  }
  if (isRecord(parsed) && parsed.success === false) {
    return typeof parsed.message === "string" ? parsed.message : "not delivered";
  }
  if (call.status !== "failed") return undefined;
  return typeof raw === "string" ? raw : "not delivered";
}

type Plan = Extract<SessionEvent, { type: "plan" }>;
type AgentMessage = Extract<SessionEvent, { type: "agent_message" }>;
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

/** An answered card's line with what the user added: `Allow · “use tabs”`. */
function withNote(answer: string, note: string | undefined): string {
  return note === undefined ? answer : `${answer} · “${note}”`;
}

function plain(value: string): Block[] {
  return value
    .split(/\n{2,}/)
    .filter((p) => p !== "")
    .map((p) => ({ type: "paragraph", spans: [{ text: p }] }));
}

const IMAGE_MARKER = /\[image (\d+)\]/g;

/**
 * A user message with its images: an image goes where its `[image N]` marker is in the text (the
 * marker gives way to it), the ones without a marker after the text.
 */
function userParts(value: string, images: string[]): Component[] {
  const parts: Component[] = [];
  const placed = new Set<number>();
  const prose = (chunk: string): void => {
    const blocks = plain(chunk.trim());
    if (blocks.length > 0) parts.push(text(blocks));
  };
  let last = 0;
  for (const match of value.matchAll(IMAGE_MARKER)) {
    const n = Number(match[1]);
    const src = images[n - 1];
    if (src === undefined || placed.has(n)) continue;
    prose(value.slice(last, match.index));
    parts.push({ type: "image", src });
    placed.add(n);
    last = match.index + match[0].length;
  }
  prose(value.slice(last));
  images.forEach((src, i) => {
    if (!placed.has(i + 1)) parts.push({ type: "image", src });
  });
  if (parts.length === 0) parts.push(text(plain(value)));
  return parts;
}

/**
 * Claude Code's adapter hands a command's output over as one markdown code block (```console …
 * ```): the output inside it.
 */
function unfenced(output: string): string {
  const lines = output.replace(/\n+$/, "").split("\n");
  const first = lines[0] ?? "";
  if (lines.length < 2 || !/^```[\w-]*$/.test(first) || lines.at(-1) !== "```") return output;
  return lines.slice(1, -1).join("\n");
}

/**
 * The shell command an execute call runs, from its input: `command` as a string, or as an argv
 * (`["bash", "-lc", "…"]` gives the script).
 */
function commandOf(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  const { command } = input;
  if (typeof command === "string") return command.trim() === "" ? undefined : command;
  if (!Array.isArray(command) || !command.every((part) => typeof part === "string")) {
    return undefined;
  }
  const argv = command as string[];
  const script = argv.length === 3 && /^-l?c$/.test(argv[1] ?? "") ? argv[2] : undefined;
  return script ?? (argv.length > 0 ? argv.join(" ") : undefined);
}

/** Whether a call's content is nothing but `text`. */
function onlyText(content: ToolCallContent[] | undefined, text: string): boolean {
  const [only, ...rest] = content ?? [];
  return rest.length === 0 && only?.type === "text" && only.text.trim() === text.trim();
}

/** Whether a call's title says no more than its command (neo cuts it to a part of the first line). */
function repeats(title: string, command: string): boolean {
  const flat = (value: string): string => value.replace(/\s+/g, " ").trim();
  const said = flat(title.replace(/^bash: /, ""));
  return said !== "" && flat(command).startsWith(said);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function connectedNotice(event: Extract<SessionEvent, { type: "app_connected" }>): string {
  if (event.reconnect === true) return `${event.app} is back`;
  if (event.previous === undefined) {
    const parts = [event.tools.join(", ")];
    if (event.components?.length) parts.push(`UI: ${event.components.join(", ")}`);
    return `${event.app} connected (v${event.v}): ${parts.join("; ")}`;
  }
  const changes: string[] = [];
  if (event.added?.length) changes.push(`+ ${event.added.join(", ")}`);
  if (event.removed?.length) changes.push(`− ${event.removed.join(", ")}`);
  if (event.components?.length) changes.push(`UI: ${event.components.join(", ")}`);
  else if (event.components) changes.push("UI: none");
  const what = changes.length > 0 ? changes.join("; ") : "schemas changed";
  return `${event.app} updated (v${event.v}): ${what}`;
}

/** Who a message is from or to: `main`, or `claude d_1a2b3c4d`. */
function partyLabel(id: string, agent: string | undefined): string {
  if (id === MAIN) return MAIN;
  return agent === undefined ? id : `${agent} ${id}`;
}

/** The first index where two block lists differ. */
function firstChange(before: Block[], after: Block[]): number {
  let i = 0;
  while (
    i < before.length &&
    i < after.length &&
    (before[i] === after[i] || JSON.stringify(before[i]) === JSON.stringify(after[i]))
  )
    i++;
  return i;
}

export class SessionView {
  private readonly items: ViewItem[] = [];
  private readonly index = new Map<string, number>();
  private readonly streams = new Map<string, Stream>();
  /** The streams not ended yet. */
  private readonly open = new Set<Stream>();
  private readonly calls = new Map<string, Call>();
  /** Connector results handed to guests as [result:ref]. */
  private readonly results = new Map<string, { tool: string; rawOutput: unknown }>();
  private readonly permissions = new Map<string, PermissionRequest>();
  private readonly choices = new Map<string, Choice>();
  /** Subagents running right now (subagent_started … subagent_ended): id → agent label. */
  private readonly windows = new Map<string, string>();
  /** Those of them that outlive a turn (started in the background). */
  private readonly background = new Set<string>();
  /** What the harness's own subagents are called, by the label of their events. */
  private readonly names = new Map<string, string>();
  /** The label of each of the harness's own subagents, by its id in the harness (SendMessage's `to`). */
  private readonly handles = new Map<string, string>();
  /** Each labelled agent's latest message so far: its stream's ref and text. */
  private readonly lastSaid = new Map<string, { ref: string; text: string }>();
  /** The calls that sent an agent a message and wait for its answer, by that agent's label. */
  private readonly asked = new Map<string, Call>();
  private readonly docs: NeoUiDocs;
  /** The open surfaces as drawn, by ref (`s-<name>`), in the order they opened. */
  private readonly surfaces = new Map<string, ViewItem>();
  /** What opened each surface: its program and place, to draw it again. */
  private readonly surfaceSources = new Map<string, SurfaceOpened>();
  /** The line of app news that is last in the chat, and what it says per app. */
  private appNews: { ref: string; parts: Map<string, string> } | undefined;
  private seq = 0;
  /** A prompt was sent: settings changed from here on show in the chat. */
  private prompted = false;
  /**
   * Inside `replay`. What replay leaves for later: the streams whose text has grown since they
   * were parsed, and the calls updated since their item was built. Each is brought up to date
   * when its item is next sent (`fresh`), so a long chat costs only what is shown of it.
   */
  private replaying = false;
  private readonly unparsed = new Set<string>();
  private readonly unbuilt = new Map<string, Call>();
  /** Told of the ops a change outside the event log makes (a neoui doc answering). */
  onOps: ((ops: ViewOp[]) => void) | undefined;

  /**
   * `only` makes a per-agent view: just that agent's work (what its labelled events say), with
   * nothing of the main conversation. Without it, the session's own view: a delegation shows as
   * the one tool item of the delegate call, and the labelled events of the subagent that runs
   * meanwhile stay out of it (they are in the log and in the agent's own view).
   */
  constructor(
    private readonly session: string,
    private readonly templates: TemplateLookup,
    private readonly log: Logger,
    private readonly only?: string,
    tools?: CallTool,
    apps?: () => AppLibrary,
    watched?: () => boolean,
  ) {
    this.docs = new NeoUiDocs((ref) => this.changed(ref), tools, apps, watched);
  }

  /** The view is dropped: the timers of its neoui docs stop. */
  dispose(): void {
    this.docs.dispose();
  }

  /** The seq of the last event applied. */
  get lastSeq(): number {
    return this.seq;
  }

  /**
   * The view as of now; with `tail`, only its last `tail` items, and in `before` how many come
   * ahead of them (`page` has those).
   */
  snapshot(tail?: number): ViewSnapshot {
    const start = tail === undefined ? 0 : Math.max(0, this.items.length - tail);
    const snapshot: ViewSnapshot = {
      session: this.session,
      seq: this.seq,
      items: this.sent(start, this.items.length),
    };
    if (start > 0) snapshot.before = start;
    if (this.only !== undefined) snapshot.agent = this.only;
    if (this.surfaces.size > 0)
      snapshot.surfaces = [...this.surfaces.values()].map((item) => structuredClone(item));
    return snapshot;
  }

  /**
   * Up to `count` items just ahead of the item `ref`, and how many come ahead of them; undefined
   * when there is no such item (any more).
   */
  page(ref: string, count: number): { items: ViewItem[]; before: number } | undefined {
    const end = this.index.get(ref);
    if (end === undefined) return undefined;
    const start = Math.max(0, end - count);
    return { items: this.sent(start, end), before: start };
  }

  /** A copy of the item `ref`, if there is one. */
  item(ref: string): ViewItem | undefined {
    const at = this.index.get(ref);
    return at === undefined ? undefined : this.sent(at, at + 1)[0];
  }

  /** Copies of items `start..end`, each brought up to date first. */
  private sent(start: number, end: number): ViewItem[] {
    const out: ViewItem[] = [];
    for (let i = start; i < end; i++) {
      const ref = this.items[i]?.ref;
      if (ref !== undefined) this.fresh(ref);
      out.push(structuredClone(this.items[i] as ViewItem));
    }
    return out;
  }

  /** What replay left of item `ref` for later, done now: its message parsed, its call's item built. */
  private fresh(ref: string): void {
    if (this.unparsed.delete(ref)) {
      const stream = this.streams.get(ref);
      if (stream !== undefined) {
        stream.blocks = this.blocksOf(stream, true);
        this.applyOp({ op: "blocks", ref, path: [], from: 0, blocks: stream.blocks });
      }
    }
    const call = this.unbuilt.get(ref);
    if (call !== undefined) {
      this.unbuilt.delete(ref);
      this.applyOp({ op: "set", item: this.toolItem(call) });
    }
  }

  /** Whether an event labelled with this agent (none: the session's own agent) is drawn. */
  private shows(agent: string | undefined): boolean {
    if (this.only !== undefined) return agent === this.only;
    return agent === undefined || ![...this.windows.values()].includes(agent);
  }

  /**
   * Builds the view from a session's log. Streamed text is only collected meanwhile and a call's
   * updates only noted: a message is parsed, a call's item built, once, when it is first sent
   * (`fresh`). Doing it all here made a long session take a second or more to open.
   */
  replay(records: Iterable<SessionEventRecord>): void {
    this.replaying = true;
    try {
      for (const record of records) this.apply(record);
    } finally {
      this.replaying = false;
    }
  }

  /** Applies one event and returns the ops that bring a pane from the previous state to this one. */
  apply(record: SessionEventRecord): ViewOp[] {
    this.seq = record.seq;
    const ops = this.opsFor(record.event, record.seq);
    for (const op of ops) {
      // A client holding only the latest items tells a new one from one it left out by this.
      if (op.op === "set" && op.item.place === undefined && !this.index.has(op.item.ref)) {
        op.appended = true;
      }
      this.applyOp(op);
    }
    return ops;
  }

  private opsFor(event: SessionEvent, seq: number): ViewOp[] {
    if (this.only !== undefined) return this.agentOps(event, seq, this.only);
    switch (event.type) {
      case "subagent_started":
        this.windows.set(event.id, event.agent);
        if (event.background === true) this.background.add(event.id);
        if (event.native === true && event.name !== undefined) {
          this.names.set(event.agent, event.name);
        }
        if (event.handle !== undefined) this.handles.set(event.handle, event.agent);
        // What it says this run, not last time.
        this.lastSaid.delete(event.agent);
        return [];
      case "subagent_ended": {
        const agent = this.windows.get(event.id);
        this.windows.delete(event.id);
        this.background.delete(event.id);
        // A run of a subagent that was sent a message: its last message is the answer.
        const said = agent === undefined ? undefined : this.lastSaid.get(agent);
        if (agent === undefined || said === undefined) return [];
        return this.answered(agent, said.text);
      }
      case "agent_message": {
        // The answer to a message the session's agent sent: it shows with that message.
        // (send_message names a delegated subagent by its id, which the answer comes from.)
        const from = [event.from, event.fromAgent].find(
          (f) => f !== undefined && this.asked.has(f),
        );
        if (from !== undefined) return this.answered(from, event.text);
        return [this.messageOp(event, seq)];
      }
      case "user_message": {
        this.prompted = true;
        // The turn that messages from other agents start: their rows are what the chat shows.
        if (event.app === MESSAGES_APP) return [];
        const children = userParts(event.shown ?? event.text, event.images ?? []);
        const component: Component =
          children.length === 1 && children[0]
            ? children[0]
            : { type: "stack", gap: "s", children };
        return [{ op: "set", item: { ref: `u${seq}`, kind: "user", label: event.app, component } }];
      }
      case "agent_message_chunk":
        if (event.agent !== undefined) {
          const ref = `m-${event.messageId}`;
          const last = this.lastSaid.get(event.agent);
          const text = last?.ref === ref ? last.text + event.text : event.text;
          this.lastSaid.set(event.agent, { ref, text });
        }
        if (!this.shows(event.agent)) return [];
        return this.stream(`m-${event.messageId}`, "agent", event.text, event.agent);
      case "agent_thought_chunk":
        if (!this.shows(event.agent)) return [];
        return this.stream(`t-${event.messageId}`, "thought", event.text, event.agent);
      case "tool_call":
        return this.toolCallOps(event);
      case "tool_call_update":
        return this.toolUpdateOps(event);
      case "tool_result":
        this.results.set(event.ref, { tool: event.tool, rawOutput: event.rawOutput });
        return [];
      case "permission_request":
        this.permissions.set(event.id, event);
        return [{ op: "set", item: this.permissionItem(event) }];
      case "permission_resolved": {
        const request = this.permissions.get(event.id);
        if (!request) return [];
        return [
          { op: "set", item: this.permissionItem(request, event.option ?? null, event.note) },
        ];
      }
      case "choice":
        this.choices.set(event.id, event);
        return [{ op: "set", item: this.choiceItem(event) }];
      case "choice_made": {
        const choice = this.choices.get(event.id);
        if (!choice) return [];
        return [{ op: "set", item: this.choiceItem(choice, event.option ?? null, event.note) }];
      }
      case "plan":
        if (!this.shows(event.agent)) return [];
        return this.planOps(event);
      case "setting_changed":
        // The approvals mode shows in the panes' status, not in the chat.
        if (event.option === "approvals") return [];
        // Before the first prompt a setting is how the session starts, which the status says.
        if (!this.prompted) return [];
        if (event.option === "title_model") {
          return [this.notice(`n${seq}`, `title model: ${event.value}`)];
        }
        // The session's harness is not one harness's setting: "harness: claude".
        if (event.option === "harness") return [this.notice(`n${seq}`, `harness: ${event.value}`)];
        return [this.notice(`n${seq}`, `${event.harness} ${event.option}: ${event.value}`)];
      case "edits_reverted": {
        const parts: string[] = [];
        if (event.paths.length > 0) parts.push(`reverted ${event.paths.join(", ")}`);
        if (event.skipped?.length) {
          parts.push(`left ${event.skipped.join(", ")} (changed since)`);
        }
        return [this.notice(`n${seq}`, parts.join("; "))];
      }
      case "notice":
        return [this.notice(`n${seq}`, event.text, event.tone ?? "muted")];
      case "context_cleared":
        return [this.notice(`n${seq}`, "context cleared: the agent starts over from here")];
      case "surface_opened": {
        const ref = surfaceRef(event.name);
        this.surfaceSources.set(ref, event);
        return [{ op: "set", item: this.surfaceItem(event) }];
      }
      case "surface_closed": {
        const ref = surfaceRef(event.name);
        if (!this.surfaceSources.delete(ref)) return [];
        this.docs.drop(ref);
        return [{ op: "remove", ref }];
      }
      case "turn_ended": {
        // A call the turn never heard the end of (it was interrupted, the harness gave no
        // result) is over with it, but a background subagent's, which runs on past the turn.
        const runsOn = new Set<string>();
        for (const [id, agent] of this.windows) if (this.background.has(id)) runsOn.add(agent);
        const closed: ViewOp[] = [];
        for (const call of this.calls.values()) {
          if (event.stopReason === "end_turn") break;
          if (call.status !== "pending" && call.status !== "in_progress") continue;
          if (call.agent !== undefined && runsOn.has(call.agent)) continue;
          call.status = "failed";
          closed.push({ op: "set", item: this.toolItem(call) });
        }
        // A subagent whose end was never logged (a crash) must not hide the next turns' work. One
        // in the background runs on past the turn.
        for (const id of [...this.windows.keys()]) {
          if (!this.background.has(id)) this.windows.delete(id);
        }
        const finished = [...closed, ...this.finishStreams()];
        if (event.stopReason === "end_turn") return finished;
        const what = event.error ?? `stopped: ${event.stopReason.replace("_", " ")}`;
        const tone = event.stopReason === "cancelled" ? "muted" : "danger";
        return [...finished, this.notice(`e${seq}`, what, tone)];
      }
      case "app_connected":
        return this.appNotice(event.app, connectedNotice(event), seq);
      case "app_disconnected":
        return this.appNotice(event.app, `${event.app} disconnected`, seq);
      case "focus_changed":
        return this.appNotice(" focus", `now in ${event.app}`, seq);
      default:
        return [];
    }
  }

  private toolCallOps(event: Extract<SessionEvent, { type: "tool_call" }>): ViewOp[] {
    if (!this.shows(event.agent)) return [];
    const prefixed = event.agent !== undefined && this.only === undefined;
    const call: Call = {
      ref: `c-${event.toolCallId}`,
      title: prefixed ? `${event.agent}: ${event.title}` : event.title,
      kind: event.kind,
      status: event.status,
      rawInput: event.rawInput,
    };
    if (event.tool !== undefined) call.tool = event.tool;
    if (event.agent !== undefined) call.agent = event.agent;
    this.calls.set(event.toolCallId, call);
    this.awaitAnswer(call);
    return [{ op: "set", item: this.toolItem(call) }];
  }

  /** The agent a sent message went to, by its label: a subagent's handle stands for it. */
  private recipient(to: string): string {
    return this.handles.get(to) ?? to;
  }

  /** A call that sent a message waits for its answer, unless it wasn't delivered. */
  private awaitAnswer(call: Call): void {
    const sent = sentMessage(call);
    if (sent === undefined || call.reply !== undefined) return;
    const to = this.recipient(sent.to);
    if (sendFailure(call) !== undefined) {
      if (this.asked.get(to) === call) this.asked.delete(to);
      return;
    }
    this.asked.set(to, call);
  }

  /** The agent that was sent a message answered: the call that sent it shows the answer. */
  private answered(agent: string, reply: string): ViewOp[] {
    const call = this.asked.get(agent);
    if (call === undefined) return [];
    this.asked.delete(agent);
    call.reply = reply.trim();
    if (this.replaying) {
      this.unbuilt.set(call.ref, call);
      return [];
    }
    this.unbuilt.delete(call.ref);
    return [{ op: "set", item: this.toolItem(call) }];
  }

  private toolUpdateOps(event: Extract<SessionEvent, { type: "tool_call_update" }>): ViewOp[] {
    const call = this.calls.get(event.toolCallId);
    if (!call) return [];
    if (event.title !== undefined) {
      const prefixed = call.agent !== undefined && this.only === undefined;
      call.title = prefixed ? `${call.agent}: ${event.title}` : event.title;
    }
    if (event.tool !== undefined) call.tool = event.tool;
    if (event.rawInput !== undefined) call.rawInput = event.rawInput;
    if (event.status !== undefined) call.status = event.status;
    if (event.content !== undefined) call.content = event.content;
    if (event.rawOutput !== undefined) call.rawOutput = event.rawOutput;
    this.awaitAnswer(call);
    if (this.replaying) {
      this.unbuilt.set(call.ref, call);
      return [];
    }
    this.unbuilt.delete(call.ref);
    return [{ op: "set", item: this.toolItem(call) }];
  }

  /** What a per-agent view draws of an event: that agent's work, and the questions it asked. */
  private agentOps(event: SessionEvent, seq: number, agent: string): ViewOp[] {
    switch (event.type) {
      case "user_message": {
        // The user addressing the guest directly: "/claude task".
        const task = event.text.trim().match(new RegExp(`^/${agent}\\s+([\\s\\S]+)$`))?.[1];
        if (task === undefined) return [];
        return [this.taskItem(`u${seq}`, task)];
      }
      case "subagent_started":
        this.windows.set(event.id, event.agent);
        if (event.agent !== agent) return [];
        return [this.taskItem(`s-${event.id}`, event.task)];
      case "agent_message": {
        // Written to this agent: like the task, a turn's input. Written by it: a row, but not its
        // final reply, which is its last message already.
        if (event.toAgent === agent) {
          const from = partyLabel(event.from, event.fromAgent);
          const item: ViewItem = {
            ref: `a${seq}`,
            kind: "user",
            label: `from ${from}`,
            component: text(parseMarkdown(event.text)),
          };
          return [{ op: "set", item }];
        }
        if (event.fromAgent !== agent || event.reply === true) return [];
        return [this.messageOp(event, seq)];
      }
      case "subagent_ended": {
        const owner = this.windows.get(event.id);
        this.windows.delete(event.id);
        if (owner !== agent) return [];
        const finished = this.finishStreams();
        if (event.stopReason === "end_turn") return finished;
        const what = event.error ?? `stopped: ${event.stopReason.replace("_", " ")}`;
        const tone = event.stopReason === "cancelled" ? "muted" : "danger";
        return [...finished, this.notice(`e${seq}`, what, tone)];
      }
      case "agent_message_chunk":
        if (event.agent !== agent) return [];
        return this.stream(`m-${event.messageId}`, "agent", event.text, undefined);
      case "agent_thought_chunk":
        if (event.agent !== agent) return [];
        return this.stream(`t-${event.messageId}`, "thought", event.text, undefined);
      case "tool_call":
        return this.toolCallOps(event);
      case "tool_call_update":
        return this.toolUpdateOps(event);
      case "tool_result":
        this.results.set(event.ref, { tool: event.tool, rawOutput: event.rawOutput });
        return [];
      case "plan":
        if (event.agent !== agent) return [];
        return this.planOps(event);
      case "turn_ended":
        return this.finishStreams();
      case "permission_request":
        if (event.agent !== agent) return [];
        this.permissions.set(event.id, event);
        return [{ op: "set", item: this.permissionItem(event) }];
      case "permission_resolved": {
        const request = this.permissions.get(event.id);
        if (!request) return [];
        return [
          { op: "set", item: this.permissionItem(request, event.option ?? null, event.note) },
        ];
      }
      default:
        return [];
    }
  }

  /** A message between agents, a compact row: `message: main → claude d_1a2b3c4d`, expands. */
  private messageOp(event: AgentMessage, seq: number): ViewOp {
    const from = partyLabel(event.from, event.fromAgent);
    const to = partyLabel(event.to, event.toAgent);
    const item: ViewItem = {
      ref: `a${seq}`,
      kind: "tool",
      label: `${event.reply === true ? "reply" : "message"}: ${from} → ${to}`,
      state: "done",
      component: text(parseMarkdown(event.text)),
    };
    return { op: "set", item };
  }

  private taskItem(ref: string, task: string): ViewOp {
    return { op: "set", item: { ref, kind: "user", label: "task", component: text(plain(task)) } };
  }

  /**
   * Apps coming, going and the agent moving between them, said in one muted line while nothing
   * else comes between: `terminal-2 disconnected · terminal is back · now in terminal`. Each app
   * (and the focus) keeps only its latest news.
   */
  private appNotice(key: string, message: string, seq: number): ViewOp[] {
    const open = this.appNews;
    if (open === undefined || this.items.at(-1)?.ref !== open.ref) {
      this.appNews = { ref: `n${seq}`, parts: new Map() };
    }
    const news = this.appNews as { ref: string; parts: Map<string, string> };
    news.parts.delete(key);
    news.parts.set(key, message);
    return [this.notice(news.ref, [...news.parts.values()].join(" · "))];
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
   * An agent asking before it acts: what it would do (a diff, text) above one button per option
   * until answered; then the one-line answer (the picked option id, or null when nobody
   * answered before the turn ended), followed by the note the user added.
   */
  private permissionItem(
    request: PermissionRequest,
    answer?: string | null,
    note?: string,
  ): ViewItem {
    // What is asked; a guest's named like its calls, `claude: …`, a harness's own subagent by
    // its name (`Explore: …`).
    const asker = request.agent === undefined ? undefined : this.names.get(request.agent);
    const label =
      this.guestAsking(request) && this.only === undefined
        ? `${asker ?? request.agent}: ${request.title}`
        : request.title;
    const details: Component[] = [];
    if (request.details?.length) {
      const items = request.details.map(({ key, value }) => ({
        key,
        value,
        tone: "accent" as const,
      }));
      details.push({ type: "kv", items });
    }
    if (answer === undefined) {
      const buttons: Component[] = request.options.map((option, i) => {
        const button: Component = {
          type: "button",
          label: option.name,
          action: "neo.permit",
          args: { id: request.id, option: option.id },
          key: String(i + 1),
          note: true,
        };
        // The option picked when nobody answers counts down to it.
        if (request.timeout?.option === option.id) button.deadline = request.timeout.at;
        return button;
      });
      const row: Component = { type: "row", gap: "s", children: buttons };
      // A command to run is shell, highlighted and broken where it is long.
      const language = request.kind === "execute" ? "bash" : undefined;
      const preview = [...details, ...this.contentComponents(request.content, language)];
      const component: Component =
        preview.length === 0 ? row : { type: "stack", gap: "s", children: [...preview, row] };
      return { ref: `p-${request.id}`, kind: "tool", label, state: "pending", component };
    }
    const option = request.options.find((o) => o.id === answer);
    const allowed = option?.kind.startsWith("allow") === true;
    const what = option ? option.name : "No answer: the turn ended.";
    const answered = text(plain(withNote(what, note)), { tone: "muted" });
    return {
      ref: `p-${request.id}`,
      kind: "tool",
      label,
      state: allowed ? "done" : "failed",
      component:
        details.length === 0
          ? answered
          : { type: "stack", gap: "s", children: [...details, answered] },
    };
  }

  /**
   * Whether a guest asks, not the session's own agent: the call it is about is a guest's, or
   * its toolCallId has a guest's `name:` prefix (the session's own guest has `name-main:`).
   */
  private guestAsking(request: PermissionRequest): boolean {
    const id = request.toolCallId;
    if (id === undefined) return false;
    const call = this.calls.get(id);
    if (call) return call.agent !== undefined;
    const prefix = /^([^:]+):/.exec(id)?.[1];
    return prefix !== undefined && !prefix.endsWith("-main");
  }

  /** A choice: one button per option (with its description, if any) until one is picked. */
  private choiceItem(choice: Choice, answer?: string | null, note?: string): ViewItem {
    const ref = `q-${choice.id}`;
    if (answer === undefined) {
      const rows: Component[] = choice.options.map((option, i) => {
        const button: Component = {
          type: "button",
          label: option.name,
          action: "neo.choose",
          args: { id: choice.id, option: option.id },
        };
        if (i < 9) button.key = String(i + 1);
        if (option.id === choice.current) button.current = true;
        // Only an answer about the agent's work takes a note for it; a setting doesn't.
        if (choice.forAgent === true) button.note = true;
        if (choice.stays === true) button.stays = true;
        if (option.input !== undefined) button.input = option.input;
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
      component: text(plain(withNote(option ? option.name : "Not answered.", note)), {
        tone: "muted",
      }),
    };
  }

  /**
   * The blocks of a streamed message; its neoui fences are programs, running ones. A chunk only
   * parses the message's end again; `all` parses all of it, for when a doc or the end changes
   * what blocks already settled say.
   */
  private blocksOf(stream: Stream, all = false): Block[] {
    if (all) stream.markdown.reset();
    if (stream.kind === "thought") return stream.markdown.parse(stream.raw);
    return stream.markdown.parse(stream.raw, (index, program, closed) =>
      this.docs.doc(stream.ref, String(index), program, !closed && !stream.ended),
    );
  }

  /** Streaming text: re-parse the whole message, send only the blocks that changed. */
  private stream(
    ref: string,
    kind: "agent" | "thought",
    chunk: string,
    agent: string | undefined,
  ): ViewOp[] {
    const existing = this.streams.get(ref);
    // A message that starts means the one before it is complete.
    const finished: ViewOp[] = [];
    if (!existing && kind === "agent") finished.push(...this.finishStreams());
    const stream: Stream = existing ?? {
      ref,
      kind,
      raw: "",
      blocks: [],
      ended: false,
      markdown: new GrowingMarkdown(),
    };
    stream.raw += chunk;
    if (!existing) {
      this.streams.set(ref, stream);
      this.open.add(stream);
    }
    if (this.replaying) {
      this.unparsed.add(ref);
      if (existing) return [];
    } else if (existing) {
      // A message replay left unparsed goes on: parsed whole first.
      this.fresh(ref);
    }
    // Replayed, its first chunk too waits to be parsed.
    let blocks: Block[] = [];
    if (!this.replaying) blocks = this.blocksOf(stream);
    const from = firstChange(stream.blocks, blocks);
    stream.blocks = blocks;
    if (!existing) {
      const component = kind === "thought" ? text(blocks, { tone: "muted" }) : text(blocks);
      const item: ViewItem = { ref, kind, component };
      if (agent !== undefined) item.label = agent;
      return [...finished, { op: "set", item }];
    }
    return [{ op: "blocks", ref, path: [], from, blocks: blocks.slice(from) }];
  }

  /** The turn is over: a neoui fence a message left open is a finished program. */
  private finishStreams(): ViewOp[] {
    const ops: ViewOp[] = [];
    for (const stream of this.open) {
      stream.ended = true;
      if (stream.kind !== "agent" || !stream.raw.includes("```")) continue;
      // Not parsed yet: it will be, as ended (`fresh`).
      if (this.unparsed.has(stream.ref)) continue;
      const before = stream.blocks;
      const blocks = this.blocksOf(stream, true);
      const from = firstChange(before, blocks);
      stream.blocks = blocks;
      if (from < before.length || from < blocks.length) {
        ops.push({ op: "blocks", ref: stream.ref, path: [], from, blocks: blocks.slice(from) });
      }
    }
    this.open.clear();
    return ops;
  }

  /**
   * A delegation reads `neo · first line of the task`, whatever the harness called its tool, and
   * so does a subagent of the harness's own (Claude Code's Agent: `Explore · its description`).
   */
  private callLabel(call: Call): string {
    if (call.agent !== undefined || !isRecord(call.rawInput)) return call.title;
    const { agent, task, subagent_type: type, description, prompt } = call.rawInput;
    let who: unknown = agent;
    let what: unknown = task;
    if (call.tool !== "delegate") {
      who = type;
      what = typeof description === "string" && description.trim() !== "" ? description : prompt;
    }
    if (typeof who !== "string" || typeof what !== "string") return call.title;
    const first = what.split("\n").find((line) => line.trim() !== "") ?? "";
    const line = first.trim().replace(/\s+/g, " ");
    return `${who} · ${line.length > 80 ? `${line.slice(0, 79)}…` : line}`;
  }

  private toolItem(call: Call): ViewItem {
    const item: ViewItem = {
      ref: call.ref,
      kind: "tool",
      label: this.callLabel(call),
      // Claude Code's adapter rarely says a call is in progress: one that has started and not
      // ended is running. (A question about it is an item of its own.)
      state: call.status === "pending" ? "running" : STATES[call.status],
      component: this.resultView(call),
    };
    const sent = sentMessage(call);
    if (sent !== undefined) return this.messageItem(call, item, sent);
    const command = call.kind === "execute" ? commandOf(call.rawInput) : undefined;
    if (command !== undefined) {
      item.command = command;
      // A title that only repeats the command (Claude Code's; neo's `bash: …`) gives way to it,
      // all but whose call it is.
      const prefixed = call.agent !== undefined && this.only === undefined;
      const title = prefixed ? call.title.slice(`${call.agent}: `.length) : call.title;
      if (repeats(title, command)) {
        if (prefixed) item.label = call.agent;
        else delete item.label;
      }
      const description = isRecord(call.rawInput) ? call.rawInput.description : undefined;
      if (typeof description === "string" && description.trim() !== "") {
        item.description = description.trim().replace(/\s+/g, " ");
      }
      // While it runs, Claude Code sends the command's description as its content: that is the
      // label, and there is no output yet.
      const running = call.status === "pending" || call.status === "in_progress";
      if (running && typeof description === "string" && onlyText(call.content, description)) {
        item.label = prefixed ? `${call.agent}: ${description}` : description;
        item.component = { type: "stack", gap: "s", children: [] };
      }
      // Ended: a test run, a diff, git's or gh's answer shown as what it is.
      if (!running) {
        const output = (call.content ?? [])
          .filter((c) => c.type === "text")
          .map((c) => unfenced(c.text))
          .join("\n");
        const recognized = recognize(command, output);
        if (recognized !== undefined) item.component = recognized;
      }
    }
    return item;
  }

  /**
   * A message sent to another agent: `message Explore`, the message in full, then that agent's
   * answer once it came (or why the message didn't arrive), instead of the tool's receipt.
   */
  private messageItem(call: Call, item: ViewItem, sent: { to: string; text: string }): ViewItem {
    const to = this.recipient(sent.to);
    const name = this.names.get(to) ?? to;
    const label = `message ${name}`;
    item.label =
      call.agent !== undefined && this.only === undefined ? `${call.agent}: ${label}` : label;
    const children: Component[] = [text(parseMarkdown(sent.text))];
    const failure = sendFailure(call);
    if (failure !== undefined) {
      children.push(text(plain(failure), { tone: "danger" }));
    } else if (call.reply !== undefined) {
      children.push(text(plain(`${name} answered:`), { tone: "muted" }));
      children.push(text(parseMarkdown(call.reply)));
    }
    item.component = { type: "stack", gap: "s", children };
    return item;
  }

  private resultView(call: Call): Component {
    const result = this.connectorResult(call);
    const template = result && this.templates(result.tool);
    if (result && template !== undefined && call.status === "completed") {
      try {
        if (isNeoUiTemplate(template)) {
          const provided = templateData(result.rawOutput);
          const ui = this.docs.doc(call.ref, "0", template.neoui, false, provided);
          if (ui.root) return { type: "neoui", ui };
          const why = ui.errors?.[0]?.message ?? "no root";
          this.log.warn(`neoui template for ${result.tool} drew nothing: ${why}`);
          return this.fallback(call);
        }
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

  /** Diffs, images and text output as components (a diff, an image, or the text as code). */
  private contentComponents(
    content: ToolCallContent[] | undefined,
    language?: string,
  ): Component[] {
    const children: Component[] = [];
    for (const item of content ?? []) {
      if (item.type === "diff") {
        children.push({
          type: "diff",
          path: item.path,
          lines: diffLines(item.oldText, item.newText),
        });
      } else if (item.type === "image") {
        children.push({ type: "image", src: item.src });
      } else if (item.text.trim() !== "") {
        const code = unfenced(item.text).replace(/\n+$/, "");
        children.push(
          language === undefined ? { type: "code", code } : { type: "code", code, language },
        );
      }
    }
    return children;
  }

  /** No template: what ACP's base data says (text output, diffs), or the structured result as kv. */
  private fallback(call: Call): Component {
    const children = this.contentComponents(call.content);
    if (children.length === 0 && isRecord(call.rawOutput)) {
      const items = Object.entries(call.rawOutput)
        .filter(([, v]) => typeof v !== "object" || v === null)
        .map(([key, value]) => ({ key, value: String(value) }));
      if (items.length > 0) children.push({ type: "kv", items });
    }
    if (children.length === 1 && children[0]) return children[0];
    return { type: "stack", gap: "s", children };
  }

  /** A surface as an item: its program drawn, in its place, apart from the chat. */
  private surfaceItem(event: SurfaceOpened): ViewItem {
    const ref = surfaceRef(event.name);
    const ui = this.docs.doc(ref, "0", event.source, false);
    return {
      ref,
      kind: "tool",
      label: event.title ?? event.name,
      place: event.place,
      component: { type: "neoui", ui },
    };
  }

  /** Doc `doc` of item `ref`, running: where typed values and $variables live. */
  program(ref: string, doc: string): Program | undefined {
    // Its message may not be parsed yet, and its docs not running.
    if (this.index.has(ref)) this.fresh(ref);
    return this.docs.program(ref, doc);
  }

  /** A neoui doc of item `ref` changed (a value was typed, a query answered): send the item again. */
  changed(ref: string): void {
    const surface = this.surfaceSources.get(ref);
    if (surface !== undefined) {
      const item = this.surfaceItem(surface);
      if (JSON.stringify(item) === JSON.stringify(this.surfaces.get(ref))) return;
      const op: ViewOp = { op: "set", item };
      this.applyOp(op);
      this.onOps?.([op]);
      return;
    }
    const at = this.index.get(ref);
    const current = at === undefined ? undefined : this.items[at];
    if (!current) return;
    let item: ViewItem | undefined;
    const stream = this.streams.get(ref);
    if (stream) {
      if (current.component.type !== "text") return;
      this.unparsed.delete(ref);
      stream.blocks = this.blocksOf(stream, true);
      item = { ...current, component: { ...current.component, blocks: stream.blocks } };
    } else {
      const call = [...this.calls.values()].find((c) => c.ref === ref);
      this.unbuilt.delete(ref);
      if (call) item = this.toolItem(call);
    }
    if (!item || JSON.stringify(item) === JSON.stringify(current)) return;
    const op: ViewOp = { op: "set", item };
    this.applyOp(op);
    this.onOps?.([op]);
  }

  private applyOp(op: ViewOp): void {
    // Surfaces are kept apart from the chat's items.
    if (op.op === "set" && op.item.place !== undefined) {
      this.surfaces.set(op.item.ref, op.item);
      return;
    }
    if (op.op === "remove" && this.surfaces.delete(op.ref)) return;
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

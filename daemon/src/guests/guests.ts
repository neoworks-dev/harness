// The ACP bridge (design §9): starts guest agents (Claude Code, Codex) through their ACP adapters,
// one guest session per suite session and guest, and streams what they do into the suite
// session. The main agent reaches them with delegate; the user with "/claude task".
import { extname } from "node:path";
import type {
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  McpServer,
  PromptResult,
} from "@suite/harness/agent";
import { formatPrompt } from "@suite/harness/context-events";
import type {
  PermissionOption,
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
  SessionEvent,
  StopReason,
  ToolCallContent,
  ToolCallStatus,
  ToolKind,
  Usage,
} from "@suite/protocol/types";
import type { Asker, PermissionRequest } from "../asker.ts";
import { BadRequestError } from "../focus.ts";
import type { ConnectorSnapshot } from "../gateway/registry.ts";
import type { Logger } from "../log.ts";
import { NotFoundError } from "../sessions.ts";
import { type FocusTarget, ToldState } from "../told.ts";
import { AcpConnection, AcpError } from "./acp.ts";
import { GUEST_PROMPT } from "./prompt.ts";

export interface GuestConfig {
  /** Shown to the main agent, e.g. "Claude Code". */
  title: string;
  /** The ACP adapter. */
  command: string[];
  env?: Record<string, string>;
  /** ask: permission requests show as buttons in the chat; allow: the first allow option wins. */
  permissions: "ask" | "allow";
}

export interface GuestRunResult extends PromptResult {
  /** The guest's last message. */
  text: string;
}

export interface GuestManagerOptions {
  configs: Record<string, GuestConfig>;
  log: Logger;
  /** Appends to a session's log. */
  append: (session: string, event: SessionEvent) => void;
  cwd: (session: string) => string;
  /** The gateway, with headers that say which guest calls. */
  mcpServers: (session: string, guest: string) => McpServer[];
  connectors: () => ConnectorSnapshot[];
  focus: (session: string) => FocusTarget | undefined;
  /** What the user chose for this guest in this session (model, effort, mode). */
  settings: (session: string, guest: string) => Partial<Record<ConfigCategory, string>>;
  /** Asks the user (permission requests). */
  asker: Asker;
  /** Whether the user lets this session's guests go ahead without asking (/approvals auto). */
  autoApprove?: (session: string) => boolean;
}

/** A conversation the adapter has stored (session/list), for /resume. */
export interface StoredConversation {
  id: string;
  title?: string;
  updatedAt?: string;
}

interface Run {
  emit: (update: AgentUpdate) => void;
  /** ACP may not send message ids: a new one starts whenever something else came in between. */
  messageId?: string;
  messageKind?: "agent_message_chunk" | "agent_thought_chunk";
  /** Text of the agent's messages, by id, in order. */
  messages: Map<string, string>;
  /** Aborted when the run ends: drops its open permission requests. */
  done: AbortController;
  /** The adapter's own title of each tool call, to describe it again once its input arrives. */
  titles: Map<string, string>;
}

interface Guest {
  name: string;
  session: string;
  conn: AcpConnection;
  acpSession: string;
  told: ToldState;
  /** One prompt at a time per guest. */
  queue: Promise<unknown>;
  run?: Run;
  /** The adapter's config options (session/new, set_config_option), by ACP config id. */
  config: Map<string, ConfigOption>;
}

const KINDS = new Set<string>([
  "read",
  "edit",
  "delete",
  "move",
  "search",
  "execute",
  "think",
  "fetch",
  "other",
]);
const STATUSES = new Set<string>(["pending", "in_progress", "completed", "failed"]);
const PLAN_STATUSES: PlanEntryStatus[] = ["pending", "in_progress", "completed"];
const PLAN_PRIORITIES: PlanEntryPriority[] = ["high", "medium", "low"];
const STOP_REASONS = new Set<string>(["end_turn", "max_tokens", "cancelled", "refusal"]);
const IMAGE_TYPES: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function count(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? Math.round(value)
    : undefined;
}

/** ACP plan entries; one without content is dropped, an unknown status counts as pending. */
function planEntries(value: unknown): PlanEntry[] {
  const entries: PlanEntry[] = [];
  if (!Array.isArray(value)) return entries;
  for (const item of value) {
    const content = isRecord(item) ? str(item.content) : undefined;
    if (!isRecord(item) || content === undefined) continue;
    const entry: PlanEntry = {
      content,
      status: PLAN_STATUSES.find((status) => status === item.status) ?? "pending",
    };
    const priority = PLAN_PRIORITIES.find((candidate) => candidate === item.priority);
    if (priority !== undefined) entry.priority = priority;
    entries.push(entry);
  }
  return entries;
}

/** ACP usage_update: context tokens used / window size, and the session's cost when in USD. */
function usageOf(update: Record<string, unknown>, agent: string): Usage | undefined {
  const usage: Usage = { type: "usage", agent };
  const used = count(update.used);
  const size = count(update.size);
  if (used !== undefined) usage.contextUsed = used;
  if (size !== undefined) usage.contextSize = size;
  if (isRecord(update.cost) && update.cost.currency === "USD") {
    const amount = update.cost.amount;
    if (typeof amount === "number" && Number.isFinite(amount) && amount >= 0) usage.cost = amount;
  }
  return Object.keys(usage).length > 2 ? usage : undefined;
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** ACP tool call content → ours (text and diffs; terminals are dropped). */
function toContent(value: unknown): ToolCallContent[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const out: ToolCallContent[] = [];
  for (const item of value) {
    if (!isRecord(item)) continue;
    if (item.type === "diff" && typeof item.path === "string" && typeof item.newText === "string") {
      const diff: ToolCallContent = { type: "diff", path: item.path, newText: item.newText };
      if (typeof item.oldText === "string") diff.oldText = item.oldText;
      out.push(diff);
    } else if (item.type === "content" && isRecord(item.content)) {
      const text = str(item.content.text);
      if (text !== undefined) out.push({ type: "text", text });
      else out.push({ type: "text", text: `[${String(item.content.type)}]` });
    }
  }
  return out;
}

/**
 * Text of an MCP result an adapter put in rawOutput instead of content (Codex:
 * {result: {content}}), so [result:ref] reaches the view.
 */
export function mcpResultText(rawOutput: unknown): ToolCallContent[] | undefined {
  const result = isRecord(rawOutput) && isRecord(rawOutput.result) ? rawOutput.result : rawOutput;
  if (!isRecord(result) || !Array.isArray(result.content)) return undefined;
  const texts = result.content.filter(isRecord).map((block) => str(block.text));
  const text = texts.filter((t) => t !== undefined).join("\n");
  return text === "" ? undefined : [{ type: "text", text }];
}

/**
 * The suite's core tools as an adapter names them (Claude Code: mcp__suite__tools_call with the
 * arguments as input; Codex: mcp.suite.tools_call with {server, tool, arguments}): show what
 * they do instead.
 */
export function describeCall(title: string, rawInput: unknown): { title: string; tool?: string } {
  const plain = title.replace(/^mcp(__|\.)suite(__|\.)/, "");
  if (!isRecord(rawInput)) return { title: plain };
  const args = isRecord(rawInput.arguments) ? rawInput.arguments : rawInput;
  if (plain.includes("tools_call")) {
    const name = str(args.name);
    if (name !== undefined) return { title: name, tool: name };
  } else if (plain.includes("tools_search")) {
    const query = str(args.query);
    if (query !== undefined) return { title: `search tools: ${query}` };
  }
  return { title: plain };
}

/** ACP config options (flat or grouped selects) → ours, keyed by config id. */
export function configOptionsOf(value: unknown): Map<string, ConfigOption> {
  const out = new Map<string, ConfigOption>();
  if (!Array.isArray(value)) return out;
  for (const option of value.filter(isRecord)) {
    const id = str(option.id);
    const category = str(option.category);
    if (
      id === undefined ||
      (category !== "model" && category !== "thought_level" && category !== "mode")
    ) {
      continue;
    }
    const values: ConfigOption["values"] = [];
    // Options may come grouped ({group, options: […]}).
    const listed: unknown[] = [];
    if (Array.isArray(option.options)) listed.push(...option.options);
    const flat: unknown[] = [];
    for (const o of listed) {
      if (isRecord(o) && Array.isArray(o.options)) flat.push(...o.options);
      else flat.push(o);
    }
    for (const entry of flat.filter(isRecord)) {
      const v = str(entry.value);
      if (v === undefined) continue;
      const item: ConfigOption["values"][number] = { value: v, name: str(entry.name) ?? v };
      const description = str(entry.description);
      if (description !== undefined) item.description = description;
      values.push(item);
    }
    const parsed: ConfigOption = { category, values };
    const current = str(option.currentValue);
    if (current !== undefined) parsed.current = current;
    out.set(id, parsed);
  }
  return out;
}

async function imageBlocks(paths: string[] | undefined): Promise<Record<string, unknown>[]> {
  const blocks: Record<string, unknown>[] = [];
  for (const path of paths ?? []) {
    const mimeType = IMAGE_TYPES[extname(path).toLowerCase()];
    if (mimeType === undefined) continue;
    const data = Buffer.from(await Bun.file(path).arrayBuffer()).toString("base64");
    blocks.push({ type: "image", mimeType, data });
  }
  return blocks;
}

export class GuestManager {
  private readonly guests = new Map<string, Promise<Guest>>();
  private readonly byAcpSession = new Map<string, Guest>();

  constructor(private readonly options: GuestManagerOptions) {}

  names(): string[] {
    return Object.keys(this.options.configs);
  }

  has(name: string): boolean {
    return Object.hasOwn(this.options.configs, name);
  }

  title(name: string): string {
    return this.options.configs[name]?.title ?? name;
  }

  /**
   * Runs one task. Updates go to `emit` as they come; the result carries the guest's last
   * message. Never rejects: failures resolve with stopReason "error".
   */
  async run(
    session: string,
    name: string,
    task: string,
    opts: { emit: (update: AgentUpdate) => void; signal?: AbortSignal; images?: string[] },
  ): Promise<GuestRunResult> {
    if (!this.has(name)) {
      return { stopReason: "error", error: `No guest named ${name}.`, text: "" };
    }
    let guest: Guest;
    try {
      guest = await this.guest(session, name);
    } catch (err) {
      return { stopReason: "error", error: `Starting ${name} failed: ${errorText(err)}`, text: "" };
    }
    const turn = guest.queue.then(() => this.prompt(guest, task, opts));
    guest.queue = turn.catch(() => {});
    return turn;
  }

  /** The guest as an AgentSession, for turns the user gives it directly (/claude task). */
  turnSession(session: string, name: string): AgentSession | undefined {
    if (!this.has(name)) return undefined;
    let controller: AbortController | undefined;
    return {
      prompt: async (params) => {
        controller = new AbortController();
        const opts = { emit: params.onUpdate, signal: controller.signal, images: params.images };
        const result = await this.run(session, name, params.text, opts);
        // The main agent hears about it with its next prompt.
        this.options.append(session, {
          type: "guest_turn",
          agent: name,
          task: params.text,
          result: result.text,
          stopReason: result.stopReason,
        });
        const out: PromptResult = { stopReason: result.stopReason };
        if (result.error !== undefined) out.error = result.error;
        return out;
      },
      cancel: () => controller?.abort(),
    };
  }

  /** The guest's settable options (starts it for this session if it isn't running). */
  async configOptions(session: string, name: string): Promise<ConfigOption[]> {
    const guest = await this.guest(session, name);
    return [...guest.config.values()];
  }

  /**
   * Sets an option on this session's guest (starting it if needed). Rejects with the adapter's
   * message when it doesn't take the value.
   */
  async configure(
    session: string,
    name: string,
    category: ConfigCategory,
    value: string,
  ): Promise<void> {
    const guest = await this.guest(session, name);
    await this.setOption(guest, category, value);
  }

  /** Conversations the guest has stored for this directory, newest first (session/list). */
  async list(name: string, cwd: string): Promise<StoredConversation[]> {
    const conn = this.spawn(name, cwd);
    try {
      await this.initialize(conn);
      const listed = await conn.request("session/list", { cwd });
      const out: StoredConversation[] = [];
      if (!isRecord(listed) || !Array.isArray(listed.sessions)) return out;
      const sessions: unknown[] = listed.sessions;
      for (const entry of sessions.filter(isRecord)) {
        const id = str(entry.sessionId);
        if (id === undefined) continue;
        const item: StoredConversation = { id };
        const title = str(entry.title);
        const updatedAt = str(entry.updatedAt);
        if (title !== undefined) item.title = title;
        if (updatedAt !== undefined) item.updatedAt = updatedAt;
        out.push(item);
      }
      return out;
    } finally {
      conn.kill();
    }
  }

  /** Continues a stored conversation as this session's guest, replacing the running one. */
  async resume(session: string, name: string, conversation: string): Promise<void> {
    const key = `${session}\0${name}`;
    const running = this.guests.get(key);
    this.guests.delete(key);
    if (running) {
      const guest = await running.catch(() => undefined);
      if (guest) {
        this.byAcpSession.delete(guest.acpSession);
        guest.conn.kill();
      }
    }
    const resumed = this.start(session, name, conversation);
    this.guests.set(key, resumed);
    resumed.catch(() => this.guests.delete(key));
    await resumed;
  }

  shutdown(): void {
    for (const guest of this.byAcpSession.values()) guest.conn.kill();
    this.guests.clear();
    this.byAcpSession.clear();
  }

  private guest(session: string, name: string): Promise<Guest> {
    if (!this.has(name)) return Promise.reject(new BadRequestError(`no guest named ${name}`));
    const key = `${session}\0${name}`;
    let guest = this.guests.get(key);
    if (!guest) {
      guest = this.start(session, name);
      this.guests.set(key, guest);
      guest.catch(() => this.guests.delete(key));
    }
    return guest;
  }

  private spawn(name: string, cwd: string): AcpConnection {
    const config = this.options.configs[name];
    if (!config) throw new NotFoundError(`no guest ${name}`);
    return AcpConnection.spawn({
      command: config.command,
      cwd,
      env: config.env,
      onNotification: (method, params) => this.onNotification(method, params),
      onRequest: (method, params) => this.onRequest(method, params),
    });
  }

  private async initialize(conn: AcpConnection): Promise<void> {
    await conn.request("initialize", {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
      clientInfo: { name: "suite", version: "0.6.0" },
    });
  }

  /** Starts the adapter and a new conversation, or resumes a stored one. */
  private async start(session: string, name: string, resume?: string): Promise<Guest> {
    const { log } = this.options;
    const cwd = this.options.cwd(session);
    let guest: Guest | undefined;
    const conn = this.spawn(name, cwd);
    void conn.exited.then((code) => {
      log.info(`${name} for ${session} exited (${code})`);
      if (!guest) return;
      this.byAcpSession.delete(guest.acpSession);
      const key = `${session}\0${name}`;
      // Only if it's still the current one (a resume replaces it).
      void this.guests.get(key)?.then((current) => current === guest && this.guests.delete(key));
    });
    try {
      await this.initialize(conn);
      const mcpServers = this.options
        .mcpServers(session, name)
        .map((server) => ({ type: "http", ...server }));
      const params: Record<string, unknown> = {
        cwd,
        mcpServers,
        // Both adapters append this to their own system prompt.
        _meta: { jetbrains: { air: { customInstructions: GUEST_PROMPT } } },
      };
      if (resume !== undefined) params.sessionId = resume;
      const created = await conn.request(
        resume === undefined ? "session/new" : "session/resume",
        params,
      );
      const acpSession = resume ?? (isRecord(created) ? str(created.sessionId) : undefined);
      if (acpSession === undefined) throw new Error("session/new returned no sessionId");
      const config = configOptionsOf(isRecord(created) ? created.configOptions : undefined);
      guest = {
        name,
        session,
        conn,
        acpSession,
        told: new ToldState(),
        queue: Promise.resolve(),
        config,
      };
      this.byAcpSession.set(acpSession, guest);
      // Only what the user chose: everything else stays the guest's own default.
      for (const [category, value] of Object.entries(this.options.settings(session, name))) {
        try {
          await this.setOption(guest, category as ConfigCategory, value);
        } catch (err) {
          log.warn(`${name}: setting ${category} to ${value} failed: ${errorText(err)}`);
        }
      }
      log.info(
        `${name} ${resume === undefined ? "started" : "resumed"} for ${session} (${acpSession})`,
      );
      return guest;
    } catch (err) {
      conn.kill();
      throw err;
    }
  }

  private async setOption(guest: Guest, category: ConfigCategory, value: string): Promise<void> {
    const entry = [...guest.config].find(([, option]) => option.category === category);
    if (!entry) {
      const name = category === "thought_level" ? "effort" : category;
      throw new BadRequestError(`${this.title(guest.name)} has no ${name} setting`);
    }
    const [configId, option] = entry;
    try {
      const result = await guest.conn.request("session/set_config_option", {
        sessionId: guest.acpSession,
        configId,
        value,
      });
      const updated = configOptionsOf(isRecord(result) ? result.configOptions : undefined);
      if (updated.size > 0) guest.config = updated;
      else option.current = value;
    } catch (err) {
      throw new BadRequestError(`${this.title(guest.name)}: ${errorText(err)}`);
    }
  }

  private async prompt(
    guest: Guest,
    task: string,
    opts: { emit: (update: AgentUpdate) => void; signal?: AbortSignal; images?: string[] },
  ): Promise<GuestRunResult> {
    const run: Run = {
      emit: opts.emit,
      messages: new Map(),
      done: new AbortController(),
      titles: new Map(),
    };
    const text = (): string => [...run.messages.values()].at(-1)?.trim() ?? "";
    if (opts.signal?.aborted) return { stopReason: "cancelled", text: "" };
    const events = guest.told.changes(this.options.connectors(), this.options.focus(guest.session));
    for (const event of events) guest.told.apply(event);
    const onAbort = (): void => {
      guest.conn.notify("session/cancel", { sessionId: guest.acpSession });
      run.done.abort();
    };
    opts.signal?.addEventListener("abort", onAbort);
    guest.run = run;
    try {
      const prompt = [
        { type: "text", text: formatPrompt(task, events) },
        ...(await imageBlocks(opts.images)),
      ];
      const result = await guest.conn.request("session/prompt", {
        sessionId: guest.acpSession,
        prompt,
      });
      const reason = isRecord(result) ? str(result.stopReason) : undefined;
      const stopReason: StopReason =
        reason !== undefined && STOP_REASONS.has(reason) ? (reason as StopReason) : "end_turn";
      return { stopReason, text: text() };
    } catch (err) {
      if (opts.signal?.aborted) return { stopReason: "cancelled", text: text() };
      return { stopReason: "error", error: `${guest.name}: ${errorText(err)}`, text: text() };
    } finally {
      opts.signal?.removeEventListener("abort", onAbort);
      run.done.abort();
      guest.run = undefined;
    }
  }
  private onNotification(method: string, params: unknown): void {
    if (method !== "session/update" || !isRecord(params) || !isRecord(params.update)) return;
    const guest = this.byAcpSession.get(str(params.sessionId) ?? "");
    const run = guest?.run;
    if (!guest || !run) return;
    const update = params.update;
    const kind = update.sessionUpdate;
    if (kind === "agent_message_chunk" || kind === "agent_thought_chunk") {
      const text = isRecord(update.content) ? str(update.content.text) : undefined;
      if (text === undefined || text === "") return;
      const given = str(update.messageId);
      if (given !== undefined) run.messageId = `${guest.name}-${given}`;
      else if (run.messageKind !== kind || run.messageId === undefined) {
        run.messageId = `${guest.name}-${crypto.randomUUID()}`;
      }
      run.messageKind = kind;
      if (kind === "agent_message_chunk") {
        run.messages.set(run.messageId, (run.messages.get(run.messageId) ?? "") + text);
      }
      run.emit({ type: kind, messageId: run.messageId, text, agent: guest.name });
      return;
    }
    // Not output of the agent: it neither ends nor splits the current message.
    if (kind === "usage_update") {
      const usage = usageOf(update, guest.name);
      if (usage) run.emit(usage);
      return;
    }
    // Anything else ends the current message.
    run.messageKind = undefined;
    if (kind === "plan") {
      if (Array.isArray(update.entries)) {
        run.emit({ type: "plan", agent: guest.name, entries: planEntries(update.entries) });
      }
      return;
    }
    const id = str(update.toolCallId);
    if (id === undefined) return;
    const toolCallId = `${guest.name}:${id}`;
    const status = str(update.status);
    const listed = toContent(update.content);
    const content = listed?.length ? listed : (mcpResultText(update.rawOutput) ?? listed);
    const rawTitle = str(update.title) ?? run.titles.get(toolCallId);
    if (rawTitle !== undefined) run.titles.set(toolCallId, rawTitle);
    if (kind === "tool_call") {
      const { title, tool } = describeCall(rawTitle ?? "tool", update.rawInput);
      const toolKind = str(update.kind);
      const call: AgentUpdate = {
        type: "tool_call",
        toolCallId,
        title,
        kind: toolKind !== undefined && KINDS.has(toolKind) ? (toolKind as ToolKind) : "other",
        status:
          status !== undefined && STATUSES.has(status) ? (status as ToolCallStatus) : "pending",
        agent: guest.name,
      };
      if (tool !== undefined) call.tool = tool;
      if (update.rawInput !== undefined) call.rawInput = update.rawInput;
      run.emit(call);
      if (content?.length) run.emit({ type: "tool_call_update", toolCallId, content });
    } else if (kind === "tool_call_update") {
      const out: AgentUpdate = { type: "tool_call_update", toolCallId };
      // Adapters stream a call's input after it started; the title can say more then.
      if (update.title !== undefined || update.rawInput !== undefined) {
        const { title, tool } = describeCall(rawTitle ?? "tool", update.rawInput);
        out.title = title;
        if (tool !== undefined) out.tool = tool;
      }
      if (update.rawInput !== undefined) out.rawInput = update.rawInput;
      if (status !== undefined && STATUSES.has(status)) out.status = status as ToolCallStatus;
      if (content !== undefined) out.content = content;
      if (update.rawOutput !== undefined) out.rawOutput = update.rawOutput;
      if (Object.keys(out).length > 2) run.emit(out);
    }
  }

  private async onRequest(method: string, params: unknown): Promise<unknown> {
    if (method !== "session/request_permission" || !isRecord(params)) {
      throw new AcpError(`${method} is not supported`, -32601);
    }
    const guest = this.byAcpSession.get(str(params.sessionId) ?? "");
    const options = this.permissionOptions(params.options);
    const cancelled = { outcome: { outcome: "cancelled" } };
    if (!guest?.run || options.length === 0) return cancelled;
    const config = this.options.configs[guest.name];
    if (config?.permissions === "allow" || this.options.autoApprove?.(guest.session) === true) {
      const allow =
        options.find((o) => o.kind === "allow_once") ??
        options.find((o) => o.kind.startsWith("allow"));
      if (allow) return { outcome: { outcome: "selected", optionId: allow.id } };
    }
    const call: Record<string, unknown> = Object.assign({}, params.toolCall);
    const request: Omit<PermissionRequest, "id"> = {
      type: "permission_request",
      agent: guest.name,
      title: describeCall(str(call.title) ?? "use a tool", call.rawInput).title,
      options,
    };
    const toolCallId = str(call.toolCallId);
    if (toolCallId !== undefined) request.toolCallId = `${guest.name}:${toolCallId}`;
    const optionId = await this.options.asker.ask(guest.session, request, guest.run.done.signal);
    return optionId === undefined ? cancelled : { outcome: { outcome: "selected", optionId } };
  }

  private permissionOptions(value: unknown): PermissionOption[] {
    if (!Array.isArray(value)) return [];
    const out: PermissionOption[] = [];
    for (const o of value) {
      if (!isRecord(o)) continue;
      const id = str(o.optionId);
      const kind = str(o.kind);
      if (id === undefined || kind === undefined) continue;
      if (!["allow_once", "allow_always", "reject_once", "reject_always"].includes(kind)) continue;
      out.push({ id, name: str(o.name) ?? id, kind: kind as PermissionOption["kind"] });
    }
    return out;
  }
}

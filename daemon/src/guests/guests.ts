// The ACP bridge (design §9): starts agents (Claude Code, Codex) through their ACP adapters and
// streams what they do into the suite session. Two roles: guests, one per suite session and
// name, that the session's agent reaches with delegate and the user with "/claude task"; and the
// session's own agent when the user runs it on that harness (/harness claude), which has no
// label in the log and gets the session's context events. pi subagents (subagent.ts) are
// delegated to the same way.
import { extname } from "node:path";
import type {
  Agent,
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  ContextEvent,
  McpServer,
  PromptResult,
  RequestPermission,
} from "@suite/harness/agent";
import { formatPrompt } from "@suite/harness/context-events";
import type {
  PermissionOption,
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
  SessionEvent,
  StopReason,
  ToolCall,
  ToolCallContent,
  ToolCallStatus,
  ToolCallUpdate,
  ToolKind,
  Usage,
} from "@suite/protocol/types";
import type { Asker, PermissionRequest } from "../asker.ts";
import { BadRequestError } from "../focus.ts";
import type { ConnectorSnapshot } from "../gateway/registry.ts";
import type { Logger } from "../log.ts";
import { NotFoundError } from "../sessions.ts";
import type { Approvals, HarnessSettings } from "../settings.ts";
import { type FocusTarget, ToldState } from "../told.ts";
import { AcpConnection, AcpError } from "./acp.ts";
import { GUEST_PROMPT, MAIN_PROMPT } from "./prompt.ts";
import type { GuestRunResult, RunOptions } from "./run.ts";
import { PI, PiSubagents, type SubagentOptions } from "./subagent.ts";

export interface GuestConfig {
  /** Shown to the main agent, e.g. "Claude Code". */
  title: string;
  /** The ACP adapter. */
  command: string[];
  env?: Record<string, string>;
  /** ask: permission requests show as buttons in the chat; allow: the first allow option wins. */
  permissions: "ask" | "allow";
}

/** Which client of the gateway an agent is (see Caller in gateway.ts). */
export interface GatewayRole {
  name: string;
  /** A subagent: it may not delegate. */
  guest?: boolean;
  /** An ACP client: it gets text-only results. */
  text?: boolean;
}

export interface GuestManagerOptions {
  configs: Record<string, GuestConfig>;
  log: Logger;
  /** Appends to a session's log. */
  append: (session: string, event: SessionEvent) => void;
  cwd: (session: string) => string;
  /** The gateway, with headers that say who calls. */
  mcpServers: (session: string, role: GatewayRole) => McpServer[];
  connectors: () => ConnectorSnapshot[];
  focus: (session: string) => FocusTarget | undefined;
  /** What the user chose for this harness in this session (model, effort, mode). */
  settings: (session: string, harness: string) => HarnessSettings;
  /** Asks the user (permission requests). */
  asker: Asker;
  /** The session's approvals mode (/approvals): auto allows everything, edits allows edits. */
  approvals?: (session: string) => Approvals;
  /**
   * A summary of the turns other harnesses ran (see handoff.ts), for an ACP harness that lost
   * its own conversation: `all` asks for every turn so far.
   */
  handoff?: (session: string, harness: string, all: boolean) => string | undefined;
  /** pi, for delegate {agent: "pi"} (its subagent sessions are made with it). */
  pi?: Agent;
  resolveAsset?: (handle: string) => string | undefined;
  /** The hook a pi subagent's own bash, write and edit ask through. */
  requestPermission?: (session: string, agent: string) => RequestPermission;
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
  /** The session's own agent, not a guest: its updates carry no agent label. */
  main: boolean;
  /** A conversation the harness doesn't carry from before (new, or its stored one is gone). */
  fresh: boolean;
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

/**
 * ACP usage_update: context tokens used / window size, and the session's cost when in USD.
 * `agent` is the guest's name; the session's own agent has none.
 */
function usageOf(update: Record<string, unknown>, agent: string | undefined): Usage | undefined {
  const usage: Usage = { type: "usage" };
  const used = count(update.used);
  const size = count(update.size);
  if (used !== undefined) usage.contextUsed = used;
  if (size !== undefined) usage.contextSize = size;
  if (isRecord(update.cost) && update.cost.currency === "USD") {
    const amount = update.cost.amount;
    if (typeof amount === "number" && Number.isFinite(amount) && amount >= 0) usage.cost = amount;
  }
  if (Object.keys(usage).length === 1) return undefined;
  if (agent !== undefined) usage.agent = agent;
  return usage;
}

/** Labels an update with the guest it is from; the session's own agent's stay unlabelled. */
function labelled<T extends object>(update: T, agent: string | undefined): T {
  return agent === undefined ? update : { ...update, agent };
}

/** The ACP conversation a harness_message of this harness last named, if any. */
export function storedConversation(history: SessionEvent[], name: string): string | undefined {
  let id: string | undefined;
  for (const event of history) {
    if (event.type !== "harness_message" || event.harness !== name) continue;
    if (isRecord(event.message)) id = str(event.message.acpSession) ?? id;
  }
  return id;
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
  /** Every running adapter (guests and sessions' own agents), to stop them with the daemon. */
  private readonly live = new Set<Guest>();
  private readonly subagents?: PiSubagents;

  constructor(private readonly options: GuestManagerOptions) {
    const { pi, requestPermission } = options;
    if (!pi) return;
    const subagents: SubagentOptions = {
      log: options.log,
      cwd: options.cwd,
      // A guest of the gateway (no delegate), but one that sees structured results.
      mcpServers: (session) => options.mcpServers(session, { name: PI, guest: true }),
      settings: (session) => options.settings(session, PI),
      connectors: options.connectors,
      focus: options.focus,
    };
    if (options.resolveAsset) subagents.resolveAsset = options.resolveAsset;
    if (requestPermission)
      subagents.requestPermission = (session) => requestPermission(session, PI);
    this.subagents = new PiSubagents(pi, subagents);
  }

  names(): string[] {
    return Object.keys(this.options.configs);
  }

  has(name: string): boolean {
    return Object.hasOwn(this.options.configs, name);
  }

  /** Everything delegate can hand a task to: pi (when there is one) and the configured guests. */
  harnesses(): string[] {
    return this.subagents ? [PI, ...this.names()] : this.names();
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
    opts: RunOptions,
  ): Promise<GuestRunResult> {
    if (name === PI && this.subagents) return this.subagents.run(session, task, opts);
    if (!this.has(name)) {
      return { stopReason: "error", error: `No guest named ${name}.`, text: "" };
    }
    let guest: Guest;
    try {
      guest = await this.guest(session, name);
    } catch (err) {
      return { stopReason: "error", error: `Starting ${name} failed: ${errorText(err)}`, text: "" };
    }
    return this.enqueue(guest, task, opts);
  }

  /** One prompt at a time per adapter. */
  private enqueue(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const turn = guest.queue.then(() => this.runWith(guest, task, opts));
    guest.queue = turn.catch(() => {});
    return turn;
  }

  /**
   * Runs the task with the settings that are meant for it alone (delegate's model and effort),
   * then puts the previous values back. A value the adapter refuses ends the task with its
   * message.
   */
  private async runWith(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const before: [ConfigCategory, string][] = [];
    const back = async (): Promise<void> => {
      for (const [category, value] of before) {
        await this.setOption(guest, category, value).catch((err: unknown) => {
          this.options.log.warn(
            `${guest.name}: putting ${category} back failed: ${errorText(err)}`,
          );
        });
      }
    };
    for (const [name, value] of Object.entries(opts.settings ?? {})) {
      const category = name as ConfigCategory;
      const current = [...guest.config.values()].find((o) => o.category === category)?.current;
      try {
        await this.setOption(guest, category, value);
      } catch (err) {
        await back();
        return { stopReason: "error", error: errorText(err), text: "" };
      }
      if (current !== undefined && current !== value) before.push([category, current]);
    }
    try {
      return await this.prompt(guest, task, opts);
    } finally {
      await back();
    }
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

  /**
   * The harness as the session's own agent (the user picked it with /harness): sessions create
   * and load it like pi. Its conversation is the adapter's own, named in the log
   * (harness_message {acpSession}) so a restarted daemon resumes it.
   */
  agent(name: string): Agent {
    return {
      name,
      newSession: (params) => this.mainSession(params.sessionId, name, undefined),
      loadSession: (params) =>
        this.mainSession(params.sessionId, name, storedConversation(params.history, name)),
    };
  }

  private async mainSession(
    session: string,
    name: string,
    stored: string | undefined,
  ): Promise<AgentSession> {
    let guest = await this.start(session, name, { main: true, resume: stored });
    let controller: AbortController | undefined;
    return {
      prompt: async (params) => {
        const abort = new AbortController();
        controller = abort;
        try {
          // The adapter may have died since the last turn: continue its conversation.
          if (!guest.conn.running) {
            guest = await this.start(session, name, { main: true, resume: guest.acpSession });
          }
        } catch (err) {
          return { stopReason: "error", error: `Starting ${name} failed: ${errorText(err)}` };
        }
        const current = guest;
        // A conversation it doesn't carry gets everything said so far, not just what it missed.
        const summary = current.fresh
          ? this.options.handoff?.(session, name, true)
          : params.summary;
        current.fresh = false;
        const opts: RunOptions = {
          emit: params.onUpdate,
          signal: abort.signal,
          images: params.images,
          events: params.pendingEvents,
        };
        if (summary !== undefined) opts.summary = summary;
        const result = await this.enqueue(current, params.text, opts);
        const out: PromptResult = { stopReason: result.stopReason };
        if (result.error !== undefined) out.error = result.error;
        return out;
      },
      cancel: () => controller?.abort(),
      configOptions: () => [...guest.config.values()],
      setConfigOption: (category, value) => this.setOption(guest, category, value),
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
    const conn = this.spawn(name, cwd, {});
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
        this.live.delete(guest);
        guest.conn.kill();
      }
    }
    const resumed = this.start(session, name, { resume: conversation });
    this.guests.set(key, resumed);
    resumed.catch(() => this.guests.delete(key));
    await resumed;
  }

  shutdown(): void {
    for (const guest of this.live) guest.conn.kill();
    this.guests.clear();
    this.live.clear();
  }

  private guest(session: string, name: string): Promise<Guest> {
    if (!this.has(name)) return Promise.reject(new BadRequestError(`no guest named ${name}`));
    const key = `${session}\0${name}`;
    let guest = this.guests.get(key);
    if (!guest) {
      guest = this.start(session, name, {});
      this.guests.set(key, guest);
      guest.catch(() => this.guests.delete(key));
    }
    return guest;
  }

  /** `target` gets the guest once it exists: the connection's updates and requests are its. */
  private spawn(name: string, cwd: string, target: { guest?: Guest }): AcpConnection {
    const config = this.options.configs[name];
    if (!config) throw new NotFoundError(`no guest ${name}`);
    return AcpConnection.spawn({
      command: config.command,
      cwd,
      env: config.env,
      onNotification: (method, params) => this.onNotification(target.guest, method, params),
      onRequest: (method, params) => this.onRequest(target.guest, method, params),
    });
  }

  private async initialize(conn: AcpConnection): Promise<void> {
    await conn.request("initialize", {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
      clientInfo: { name: "suite", version: "0.6.0" },
    });
  }

  /**
   * Starts the adapter and a new conversation, or resumes a stored one. The session's own agent
   * falls back to a new conversation when the stored one can't be resumed.
   */
  private async start(
    session: string,
    name: string,
    opts: { main?: boolean; resume?: string },
  ): Promise<Guest> {
    const { main = false, resume } = opts;
    if (main && resume !== undefined) {
      try {
        return await this.open(session, name, main, resume);
      } catch (err) {
        this.options.log.warn(
          `${name} for ${session}: resuming ${resume} failed (${errorText(err)}); starting anew`,
        );
      }
    }
    return this.open(session, name, main, main ? undefined : resume);
  }

  private async open(
    session: string,
    name: string,
    main: boolean,
    resume: string | undefined,
  ): Promise<Guest> {
    const { log } = this.options;
    const cwd = this.options.cwd(session);
    const target: { guest?: Guest } = {};
    const conn = this.spawn(name, cwd, target);
    void conn.exited.then((code) => {
      log.info(`${name} for ${session} exited (${code})`);
      const { guest } = target;
      if (!guest) return;
      this.live.delete(guest);
      if (main) return;
      const key = `${session}\0${name}`;
      // Only if it's still the current one (a resume replaces it).
      void this.guests.get(key)?.then((current) => current === guest && this.guests.delete(key));
    });
    try {
      await this.initialize(conn);
      const mcpServers = this.options
        .mcpServers(session, { name, guest: !main, text: true })
        .map((server) => ({ type: "http", ...server }));
      const params: Record<string, unknown> = {
        cwd,
        mcpServers,
        // Both adapters append this to their own system prompt.
        _meta: { jetbrains: { air: { customInstructions: main ? MAIN_PROMPT : GUEST_PROMPT } } },
      };
      if (resume !== undefined) params.sessionId = resume;
      const created = await conn.request(
        resume === undefined ? "session/new" : "session/resume",
        params,
      );
      const acpSession = resume ?? (isRecord(created) ? str(created.sessionId) : undefined);
      if (acpSession === undefined) throw new Error("session/new returned no sessionId");
      const config = configOptionsOf(isRecord(created) ? created.configOptions : undefined);
      const guest: Guest = {
        name,
        session,
        main,
        fresh: main && resume === undefined,
        conn,
        acpSession,
        told: new ToldState(),
        queue: Promise.resolve(),
        config,
      };
      target.guest = guest;
      this.live.add(guest);
      // Only what the user chose: everything else stays the guest's own default.
      for (const [category, value] of Object.entries(this.options.settings(session, name))) {
        try {
          await this.setOption(guest, category as ConfigCategory, value);
        } catch (err) {
          log.warn(`${name}: setting ${category} to ${value} failed: ${errorText(err)}`);
        }
      }
      if (main && resume === undefined) {
        // So a restarted daemon continues this conversation.
        this.options.append(session, {
          type: "harness_message",
          harness: name,
          message: { acpSession },
        });
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

  private async prompt(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const run: Run = {
      emit: opts.emit,
      messages: new Map(),
      done: new AbortController(),
      titles: new Map(),
    };
    const text = (): string => [...run.messages.values()].at(-1)?.trim() ?? "";
    if (opts.signal?.aborted) return { stopReason: "cancelled", text: "" };
    // What this adapter wasn't told yet about the connectors and focus, then what the session
    // wants the agent to hear (guests' turns, reverted edits).
    const events: ContextEvent[] = guest.told.changes(
      this.options.connectors(),
      this.options.focus(guest.session),
    );
    for (const event of events) guest.told.apply(event);
    for (const event of opts.events ?? []) {
      if (event.type === "guest_turn" || event.type === "edits_reverted") events.push(event);
    }
    const onAbort = (): void => {
      guest.conn.notify("session/cancel", { sessionId: guest.acpSession });
      run.done.abort();
    };
    opts.signal?.addEventListener("abort", onAbort);
    guest.run = run;
    try {
      const prompt = [
        { type: "text", text: formatPrompt(task, events, undefined, opts.summary) },
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

  private onNotification(guest: Guest | undefined, method: string, params: unknown): void {
    if (method !== "session/update" || !isRecord(params) || !isRecord(params.update)) return;
    const run = guest?.run;
    if (!guest || !run) return;
    // The session's own agent is unlabelled; ids are prefixed so they stay unique in the log.
    const agent = guest.main ? undefined : guest.name;
    const prefix = guest.main ? `${guest.name}-main` : guest.name;
    const update = params.update;
    const kind = update.sessionUpdate;
    if (kind === "agent_message_chunk" || kind === "agent_thought_chunk") {
      const text = isRecord(update.content) ? str(update.content.text) : undefined;
      if (text === undefined || text === "") return;
      const given = str(update.messageId);
      if (given !== undefined) run.messageId = `${prefix}-${given}`;
      else if (run.messageKind !== kind || run.messageId === undefined) {
        run.messageId = `${prefix}-${crypto.randomUUID()}`;
      }
      run.messageKind = kind;
      if (kind === "agent_message_chunk") {
        run.messages.set(run.messageId, (run.messages.get(run.messageId) ?? "") + text);
      }
      run.emit(labelled({ type: kind, messageId: run.messageId, text }, agent));
      return;
    }
    // Not output of the agent: it neither ends nor splits the current message.
    if (kind === "usage_update") {
      const usage = usageOf(update, agent);
      if (usage) run.emit(usage);
      return;
    }
    // Anything else ends the current message.
    run.messageKind = undefined;
    if (kind === "plan") {
      if (Array.isArray(update.entries)) {
        run.emit(labelled({ type: "plan", entries: planEntries(update.entries) }, agent));
      }
      return;
    }
    const id = str(update.toolCallId);
    if (id === undefined) return;
    const toolCallId = `${prefix}:${id}`;
    const status = str(update.status);
    const listed = toContent(update.content);
    const content = listed?.length ? listed : (mcpResultText(update.rawOutput) ?? listed);
    const rawTitle = str(update.title) ?? run.titles.get(toolCallId);
    if (rawTitle !== undefined) run.titles.set(toolCallId, rawTitle);
    if (kind === "tool_call") {
      const { title, tool } = describeCall(rawTitle ?? "tool", update.rawInput);
      const toolKind = str(update.kind);
      const call: ToolCall = {
        type: "tool_call",
        toolCallId,
        title,
        kind: toolKind !== undefined && KINDS.has(toolKind) ? (toolKind as ToolKind) : "other",
        status:
          status !== undefined && STATUSES.has(status) ? (status as ToolCallStatus) : "pending",
      };
      if (agent !== undefined) call.agent = agent;
      if (tool !== undefined) call.tool = tool;
      if (update.rawInput !== undefined) call.rawInput = update.rawInput;
      run.emit(call);
      if (content?.length) run.emit({ type: "tool_call_update", toolCallId, content });
    } else if (kind === "tool_call_update") {
      const out: ToolCallUpdate = { type: "tool_call_update", toolCallId };
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

  private async onRequest(
    guest: Guest | undefined,
    method: string,
    params: unknown,
  ): Promise<unknown> {
    if (method !== "session/request_permission" || !isRecord(params)) {
      throw new AcpError(`${method} is not supported`, -32601);
    }
    const options = this.permissionOptions(params.options);
    const cancelled = { outcome: { outcome: "cancelled" } };
    if (!guest?.run || options.length === 0) return cancelled;
    const call: Record<string, unknown> = Object.assign({}, params.toolCall);
    const content = toContent(call.content);
    const callKind = str(call.kind);
    const kind: ToolKind | undefined =
      callKind !== undefined && KINDS.has(callKind) ? (callKind as ToolKind) : undefined;
    const isEdit = kind === "edit" || content?.some((c) => c.type === "diff") === true;
    // The same mapping for guests and the session's own agent.
    const config = this.options.configs[guest.name];
    const mode = this.options.approvals?.(guest.session);
    if (config?.permissions === "allow" || mode === "auto" || (mode === "edits" && isEdit)) {
      const allow =
        options.find((o) => o.kind === "allow_once") ??
        options.find((o) => o.kind.startsWith("allow"));
      if (allow) return { outcome: { outcome: "selected", optionId: allow.id } };
    }
    const request: Omit<PermissionRequest, "id"> = {
      type: "permission_request",
      agent: guest.name,
      title: describeCall(str(call.title) ?? "use a tool", call.rawInput).title,
      options,
    };
    if (kind !== undefined) request.kind = kind;
    if (content?.length) request.content = content;
    const toolCallId = str(call.toolCallId);
    if (toolCallId !== undefined) {
      request.toolCallId = `${guest.main ? `${guest.name}-main` : guest.name}:${toolCallId}`;
    }
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

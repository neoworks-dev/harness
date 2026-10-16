// The ACP bridge (design §9): starts guest agents (Claude Code, Codex) through their ACP adapters,
// one guest session per suite session and guest, and streams what they do into the suite
// session. The main agent reaches them with delegate; the user with "/claude task".
import { extname } from "node:path";
import type { AgentSession, AgentUpdate, McpServer, PromptResult } from "@suite/harness/agent";
import { formatPrompt } from "@suite/harness/context-events";
import type {
  PermissionOption,
  SessionEvent,
  StopReason,
  ToolCallContent,
  ToolCallStatus,
  ToolKind,
} from "@suite/protocol/types";
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
  /** Value for the adapter's "model" config option, e.g. haiku. */
  model?: string;
  /** Value for its "mode" config option (permission mode). */
  mode?: string;
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
}

interface Run {
  emit: (update: AgentUpdate) => void;
  /** ACP may not send message ids: a new one starts whenever something else came in between. */
  messageId?: string;
  messageKind?: "agent_message_chunk" | "agent_thought_chunk";
  /** Text of the agent's messages, by id, in order. */
  messages: Map<string, string>;
  /** Permission requests waiting for an answer. */
  permissions: Set<string>;
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
}

interface PendingPermission {
  session: string;
  guest: Guest;
  options: PermissionOption[];
  resolve: (optionId: string | undefined) => void;
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
  private readonly permissions = new Map<string, PendingPermission>();

  constructor(private readonly options: GuestManagerOptions) {}

  names(): string[] {
    return Object.keys(this.options.configs);
  }

  has(name: string): boolean {
    return Object.hasOwn(this.options.configs, name);
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

  /** A button press on a permission_request (action suite.permit). */
  permit(session: string, id: string, option: string, by: string): void {
    const pending = this.permissions.get(id);
    if (!pending || pending.session !== session) {
      throw new BadRequestError(`no open permission request ${id}`);
    }
    if (!pending.options.some((o) => o.id === option)) {
      throw new BadRequestError(`permission request ${id} has no option ${option}`);
    }
    this.answer(id, option, by);
  }

  shutdown(): void {
    for (const id of [...this.permissions.keys()]) this.answer(id, undefined);
    for (const guest of this.byAcpSession.values()) guest.conn.kill();
    this.guests.clear();
    this.byAcpSession.clear();
  }

  private guest(session: string, name: string): Promise<Guest> {
    const key = `${session}\0${name}`;
    let guest = this.guests.get(key);
    if (!guest) {
      guest = this.start(session, name);
      this.guests.set(key, guest);
      guest.catch(() => this.guests.delete(key));
    }
    return guest;
  }

  private async start(session: string, name: string): Promise<Guest> {
    const config = this.options.configs[name];
    if (!config) throw new NotFoundError(`no guest ${name}`);
    const { log } = this.options;
    const cwd = this.options.cwd(session);
    let guest: Guest | undefined;
    const conn = AcpConnection.spawn({
      command: config.command,
      cwd,
      env: config.env,
      onNotification: (method, params) => this.onNotification(method, params),
      onRequest: (method, params) => this.onRequest(method, params),
    });
    void conn.exited.then((code) => {
      log.info(`${name} for ${session} exited (${code})`);
      this.guests.delete(`${session}\0${name}`);
      if (guest) this.byAcpSession.delete(guest.acpSession);
    });
    try {
      await conn.request("initialize", {
        protocolVersion: 1,
        clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
        clientInfo: { name: "suite", version: "0.6.0" },
      });
      const mcpServers = this.options
        .mcpServers(session, name)
        .map((server) => ({ type: "http", ...server }));
      const created = await conn.request("session/new", {
        cwd,
        mcpServers,
        // Both adapters append this to their own system prompt.
        _meta: { jetbrains: { air: { customInstructions: GUEST_PROMPT } } },
      });
      const acpSession = isRecord(created) ? str(created.sessionId) : undefined;
      if (acpSession === undefined) throw new Error("session/new returned no sessionId");
      guest = { name, session, conn, acpSession, told: new ToldState(), queue: Promise.resolve() };
      this.byAcpSession.set(acpSession, guest);
      for (const [configId, value] of [
        ["model", config.model],
        ["mode", config.mode],
      ] as const) {
        if (value === undefined) continue;
        try {
          await conn.request("session/set_config_option", {
            sessionId: acpSession,
            configId,
            value,
          });
        } catch (err) {
          log.warn(`${name}: setting ${configId} to ${value} failed: ${errorText(err)}`);
        }
      }
      log.info(`${name} started for ${session} (${acpSession})`);
      return guest;
    } catch (err) {
      conn.kill();
      throw err;
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
      permissions: new Set(),
      titles: new Map(),
    };
    const text = (): string => [...run.messages.values()].at(-1)?.trim() ?? "";
    if (opts.signal?.aborted) return { stopReason: "cancelled", text: "" };
    const events = guest.told.changes(this.options.connectors(), this.options.focus(guest.session));
    for (const event of events) guest.told.apply(event);
    const onAbort = (): void => {
      guest.conn.notify("session/cancel", { sessionId: guest.acpSession });
      for (const id of run.permissions) this.answer(id, undefined);
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
      for (const id of run.permissions) this.answer(id, undefined);
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
    // Anything else ends the current message.
    run.messageKind = undefined;
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
    if (config?.permissions === "allow") {
      const allow =
        options.find((o) => o.kind === "allow_once") ??
        options.find((o) => o.kind.startsWith("allow"));
      if (allow) return { outcome: { outcome: "selected", optionId: allow.id } };
    }
    const call: Record<string, unknown> = Object.assign({}, params.toolCall);
    const id = `p_${crypto.randomUUID().slice(0, 8)}`;
    const optionId = await new Promise<string | undefined>((resolve) => {
      this.permissions.set(id, { session: guest.session, guest, options, resolve });
      guest.run?.permissions.add(id);
      const request: SessionEvent = {
        type: "permission_request",
        id,
        agent: guest.name,
        title: describeCall(str(call.title) ?? "use a tool", call.rawInput).title,
        options,
      };
      const toolCallId = str(call.toolCallId);
      if (toolCallId !== undefined) request.toolCallId = `${guest.name}:${toolCallId}`;
      this.options.append(guest.session, request);
    });
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

  /** Settles a permission request: the picked option, or none (cancelled). */
  private answer(id: string, option: string | undefined, by?: string): void {
    const pending = this.permissions.get(id);
    if (!pending) return;
    this.permissions.delete(id);
    pending.guest.run?.permissions.delete(id);
    const event: SessionEvent = { type: "permission_resolved", id };
    if (option !== undefined) event.option = option;
    if (by !== undefined) event.by = by;
    this.options.append(pending.session, event);
    pending.resolve(option);
  }
}

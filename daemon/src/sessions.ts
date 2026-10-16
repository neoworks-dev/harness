// Session manager: one turn at a time per session, inputs queued behind it. Talks to the agent only
// through the ACP-style interface in @suite/harness/agent.
import type {
  Agent,
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  ContextEvent,
  McpServer,
  NewSessionParams,
  RequestPermission,
} from "@suite/harness/agent";
import type { SessionEvent, SessionInfo, TurnEnded } from "@suite/protocol/types";
import type { ConnectorSnapshot } from "./gateway/registry.ts";
import type { Logger } from "./log.ts";
import type { SessionRow, SessionStore } from "./store.ts";
import { type FocusTarget, ToldState } from "./told.ts";

export type { FocusTarget } from "./told.ts";

export class NotFoundError extends Error {}

/** A request that can't work as asked (the message says why). */
export class BadRequestError extends Error {}

interface QueuedInput {
  app: string;
  text: string;
  images?: string[];
}

/** In-memory state of a session that has been used since the daemon started. */
interface Runtime {
  queue: QueuedInput[];
  running: boolean;
  agentSession?: AgentSession;
  /** Whoever runs the current turn: the agent session, or a guest the user addressed. */
  active?: AgentSession;
  /** Set when cancel() arrives while the agent session is still loading. */
  cancelRequested: boolean;
  /** Connector events not yet delivered to the agent. */
  pending: ContextEvent[];
  /** What this session's log has told the agent. */
  told: ToldState;
  /** Resolves when the current drain loop finishes. */
  drained: Promise<void>;
}

const CONTEXT_TYPES = new Set<SessionEvent["type"]>([
  "app_connected",
  "app_disconnected",
  "focus_changed",
  "guest_turn",
  "edits_reverted",
]);

function isContextEvent(event: SessionEvent): event is ContextEvent {
  return CONTEXT_TYPES.has(event.type);
}

function userMessage(input: QueuedInput): SessionEvent {
  const event: SessionEvent = { type: "user_message", app: input.app, text: input.text };
  if (input.images) event.images = input.images;
  return event;
}

export function newSessionId(): string {
  const alphabet = "0123456789abcdefghjkmnpqrstvwxyz";
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  return `s_${Array.from(bytes, (b) => alphabet[b % 32]).join("")}`;
}

export interface SessionManagerOptions {
  defaultCwd: string;
  log: Logger;
  /** Open connectors right now; each turn starts by telling the session what changed. */
  connectors?: () => ConnectorSnapshot[];
  /** Where the agent is, if `sessionId` is the current session; announced as focus_changed. */
  focus?: (sessionId: string) => FocusTarget | undefined;
  /** MCP servers handed to the agent for a session (the gateway). */
  mcpServers?: (sessionId: string) => McpServer[];
  /** Resolves asset://… handles for the agent's file tools. */
  resolveAsset?: (handle: string) => string | undefined;
  /** More about a session for its info: preview, lines changed (see summaries.ts). */
  describe?: (sessionId: string) => Partial<SessionInfo>;
  /**
   * A guest the user addresses directly with "/name task" (e.g. /claude), for this session;
   * undefined when there is none by that name. Its prompt() gets the task as text.
   */
  guest?: (sessionId: string, name: string) => AgentSession | undefined;
  /** What the user chose for the agent in this session (model, effort…). */
  settings?: (sessionId: string) => Partial<Record<ConfigCategory, string>>;
  /** The hook the agent's own bash, write and edit ask through (see permissions.ts). */
  requestPermission?: (sessionId: string) => RequestPermission;
}

/** "/claude fix it" → claude, "fix it". */
function guestCommand(text: string): { name: string; task: string } | undefined {
  const match = /^\/([a-z][\w-]*)(?:\s+([\s\S]*))?$/.exec(text.trim());
  if (!match?.[1]) return undefined;
  return { name: match[1], task: match[2]?.trim() ?? "" };
}

export class SessionManager {
  private readonly runtimes = new Map<string, Runtime>();
  private readonly stateListeners: ((sessionId: string) => void)[] = [];

  constructor(
    private readonly store: SessionStore,
    private readonly agent: Agent,
    private readonly options: SessionManagerOptions,
  ) {
    // Connector events can be appended by anyone (Phase 2: presence); collect them for the next prompt.
    store.onAppend((record) => {
      if (!isContextEvent(record.event)) return;
      const rt = this.runtimes.get(record.session);
      if (!rt) return;
      rt.pending.push(record.event);
      rt.told.apply(record.event);
    });
  }

  create(params: { cwd?: string; title?: string }): SessionInfo {
    const row = this.store.createSession({
      id: newSessionId(),
      cwd: params.cwd ?? this.options.defaultCwd,
      title: params.title,
    });
    return this.toInfo(row);
  }

  info(id: string): SessionInfo | undefined {
    const row = this.store.getSession(id);
    return row && this.toInfo(row);
  }

  list(): SessionInfo[] {
    return this.store.listSessions().map((row) => this.toInfo(row));
  }

  /** Queues input. When this returns, info() already shows the session running or queued. */
  input(sessionId: string, app: string, text: string, images?: string[]): void {
    const rt = this.runtime(sessionId);
    rt.queue.push(images?.length ? { app, text, images } : { app, text });
    if (!rt.running) {
      rt.running = true;
      rt.drained = this.drain(sessionId, rt);
    }
    this.stateChanged(sessionId);
  }

  /** The agent's settable options for this session (loads its agent session if needed). */
  async configOptions(sessionId: string): Promise<ConfigOption[]> {
    const agentSession = await this.agentSession(sessionId);
    return agentSession.configOptions?.() ?? [];
  }

  /** Sets one for the session's following turns; rejects if the agent doesn't take it. */
  async configure(sessionId: string, category: ConfigCategory, value: string): Promise<void> {
    const agentSession = await this.agentSession(sessionId);
    if (!agentSession.setConfigOption) {
      throw new BadRequestError(`${this.agent.name} has no settings`);
    }
    try {
      await agentSession.setConfigOption(category, value);
    } catch (err) {
      throw new BadRequestError(err instanceof Error ? err.message : String(err));
    }
  }

  private async agentSession(sessionId: string): Promise<AgentSession> {
    const rt = this.runtime(sessionId);
    rt.agentSession ??= await this.loadAgentSession(sessionId);
    return rt.agentSession;
  }

  /** Cancels the running turn; queued inputs still run. */
  cancel(sessionId: string): void {
    const rt = this.runtime(sessionId);
    if (!rt.running) return;
    if (rt.active) rt.active.cancel();
    else rt.cancelRequested = true;
  }

  /** Resolves once the session has no running or queued turns. */
  async idle(sessionId: string): Promise<void> {
    const rt = this.runtimes.get(sessionId);
    while (rt?.running) await rt.drained;
  }

  /** Cancels every running turn and waits for them to finish (or the timeout). */
  async shutdown(timeoutMs: number): Promise<void> {
    const waits: Promise<void>[] = [];
    for (const [id, rt] of this.runtimes) {
      rt.queue.length = 0;
      if (!rt.running) continue;
      this.cancel(id);
      waits.push(rt.drained);
    }
    await Promise.race([Promise.all(waits), Bun.sleep(timeoutMs)]);
  }

  /**
   * After a crash the log can end inside a turn. Close those turns so views and the next prompt
   * see a consistent log; the agent repairs its own context on load.
   */
  recoverInterruptedTurns(): void {
    for (const row of this.store.listSessions()) {
      const lastInput = this.store.lastSeqOfType(row.id, "user_message");
      if (lastInput > this.store.lastSeqOfType(row.id, "turn_ended")) {
        this.store.append(row.id, {
          type: "turn_ended",
          stopReason: "error",
          error: "Interrupted: the daemon stopped during this turn.",
        });
        this.options.log.info(`closed interrupted turn in ${row.id}`);
      }
    }
  }

  private runtime(sessionId: string): Runtime {
    const existing = this.runtimes.get(sessionId);
    if (existing) return existing;
    if (!this.store.getSession(sessionId)) throw new NotFoundError(`no session ${sessionId}`);
    // Connector events since the last prompt are still pending after a restart.
    const lastInput = this.store.lastSeqOfType(sessionId, "user_message");
    const pending = this.store
      .read(sessionId, lastInput + 1)
      .map((r) => r.event)
      .filter(isContextEvent);
    const told = new ToldState();
    const types: SessionEvent["type"][] = ["app_connected", "app_disconnected", "focus_changed"];
    for (const r of this.store.readTypes(sessionId, types)) told.apply(r.event);
    const rt: Runtime = {
      queue: [],
      running: false,
      cancelRequested: false,
      pending,
      told,
      drained: Promise.resolve(),
    };
    this.runtimes.set(sessionId, rt);
    return rt;
  }

  private async drain(sessionId: string, rt: Runtime): Promise<void> {
    try {
      for (let input = rt.queue.shift(); input; input = rt.queue.shift()) {
        await this.runTurn(sessionId, rt, input);
      }
    } finally {
      rt.running = false;
      this.stateChanged(sessionId);
    }
  }

  private async runTurn(sessionId: string, rt: Runtime, input: QueuedInput): Promise<void> {
    const command = guestCommand(input.text);
    const guest = command && this.options.guest?.(sessionId, command.name);
    if (command && guest) {
      await this.runGuestTurn(sessionId, rt, input, guest, command.task);
      return;
    }
    const ended: TurnEnded = { type: "turn_ended", stopReason: "error" };
    let userLogged = false;
    try {
      const agentSession = await this.agentSession(sessionId);
      this.announceConnectors(sessionId, rt);
      const pendingEvents = rt.pending.splice(0);
      this.store.append(sessionId, userMessage(input));
      userLogged = true;
      if (rt.cancelRequested) {
        ended.stopReason = "cancelled";
      } else {
        rt.active = agentSession;
        Object.assign(ended, await this.prompt(sessionId, agentSession, input, pendingEvents));
      }
    } catch (err) {
      // prompt() never rejects by contract, but loading the agent session can fail.
      ended.error = err instanceof Error ? err.message : String(err);
      this.options.log.error(`${sessionId}: turn failed: ${ended.error}`);
      if (!userLogged) this.store.append(sessionId, userMessage(input));
    } finally {
      rt.cancelRequested = false;
      rt.active = undefined;
    }
    this.store.append(sessionId, ended);
  }

  /**
   * "/claude task": the guest runs the turn instead of the agent. The agent's pending events
   * stay pending; the guest tells it what happened (guest_turn) for its next prompt.
   */
  private async runGuestTurn(
    sessionId: string,
    rt: Runtime,
    input: QueuedInput,
    guest: AgentSession,
    task: string,
  ): Promise<void> {
    this.store.append(sessionId, userMessage(input));
    const ended: TurnEnded = { type: "turn_ended", stopReason: "cancelled" };
    if (task === "") {
      Object.assign(ended, { stopReason: "error", error: `Usage: ${input.text.trim()} <task>` });
    } else if (!rt.cancelRequested) {
      rt.active = guest;
      try {
        const turn = { ...input, text: task };
        Object.assign(ended, await this.prompt(sessionId, guest, turn, []));
      } finally {
        rt.active = undefined;
      }
    }
    rt.cancelRequested = false;
    this.store.append(sessionId, ended);
  }

  private async prompt(
    sessionId: string,
    agentSession: AgentSession,
    input: QueuedInput,
    pendingEvents: ContextEvent[],
  ): Promise<Omit<TurnEnded, "type">> {
    let open = true;
    const onUpdate = (update: AgentUpdate): void => {
      if (open) this.store.append(sessionId, update);
      else this.options.log.warn(`${sessionId}: dropped ${update.type} sent after the turn ended`);
    };
    try {
      const result = await agentSession.prompt({ ...input, pendingEvents, onUpdate });
      const out: Omit<TurnEnded, "type"> = { stopReason: result.stopReason };
      if (result.error !== undefined) out.error = result.error;
      return out;
    } finally {
      open = false;
    }
  }

  /**
   * Appends app_connected / app_disconnected for whatever changed since this session was last
   * told (by version), so the next prompt carries exactly the difference. Idle sessions get no
   * writes; a quick close-and-reopen between two prompts produces nothing.
   */
  private announceConnectors(sessionId: string, rt: Runtime): void {
    const current = this.options.connectors?.();
    if (!current) return;
    // Appending applies each event to rt.told (see the onAppend hook).
    for (const event of rt.told.changes(current, this.options.focus?.(sessionId))) {
      this.store.append(sessionId, event);
    }
  }

  private async loadAgentSession(sessionId: string): Promise<AgentSession> {
    const row = this.store.getSession(sessionId);
    if (!row) throw new NotFoundError(`no session ${sessionId}`);
    const mcpServers = this.options.mcpServers?.(sessionId) ?? [];
    const params: NewSessionParams = { sessionId, cwd: row.cwd, mcpServers };
    if (this.options.resolveAsset) params.resolveAsset = this.options.resolveAsset;
    if (this.options.requestPermission) {
      params.requestPermission = this.options.requestPermission(sessionId);
    }
    const settings = this.options.settings?.(sessionId);
    if (settings && Object.keys(settings).length > 0) params.settings = settings;
    if (row.lastSeq === 0) return this.agent.newSession(params);
    const history = this.store.read(sessionId).map((r) => r.event);
    return this.agent.loadSession({ ...params, history });
  }

  private toInfo(row: SessionRow): SessionInfo {
    const rt = this.runtimes.get(row.id);
    const info: SessionInfo = {
      id: row.id,
      cwd: row.cwd,
      createdAt: row.createdAt,
      lastActivity: row.lastActivity,
      lastSeq: row.lastSeq,
      state: rt?.running ? "running" : "idle",
      queued: rt?.queue.length ?? 0,
    };
    if (row.title !== undefined) info.title = row.title;
    if (this.options.describe) Object.assign(info, this.options.describe(row.id));
    return info;
  }

  /** Called when a session starts or stops running, or its queue changes. */
  onStateChange(listener: (sessionId: string) => void): void {
    this.stateListeners.push(listener);
  }

  private stateChanged(sessionId: string): void {
    for (const listener of this.stateListeners) listener(sessionId);
  }
}

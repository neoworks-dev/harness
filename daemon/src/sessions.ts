// Session manager: one turn at a time per session, inputs queued behind it. Talks to the agent only
// through the ACP-style interface in @suite/harness/agent.
import type {
  Agent,
  AgentSession,
  AgentUpdate,
  ContextEvent,
  McpServer,
  NewSessionParams,
} from "@suite/harness/agent";
import type { AppConnected, SessionEvent, SessionInfo, TurnEnded } from "@suite/protocol/types";
import type { ConnectorSnapshot } from "./gateway/registry.ts";
import type { Logger } from "./log.ts";
import type { SessionRow, SessionStore } from "./store.ts";

export class NotFoundError extends Error {}

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
  /** Set when cancel() arrives while the agent session is still loading. */
  cancelRequested: boolean;
  /** Connector events not yet delivered to the agent. */
  pending: ContextEvent[];
  /** What this session's log says is open: app → version it was told about. */
  told: Map<string, number>;
  /** Every connector this session ever heard of (open or not): the last version and tools. */
  seen: Map<string, { v: number; tools: string[] }>;
  /** The last focus_changed in this session's log. */
  toldFocus?: FocusTarget;
  /** Resolves when the current drain loop finishes. */
  drained: Promise<void>;
}

const CONTEXT_TYPES = new Set<SessionEvent["type"]>([
  "app_connected",
  "app_disconnected",
  "focus_changed",
]);

function isContextEvent(event: SessionEvent): event is ContextEvent {
  return CONTEXT_TYPES.has(event.type);
}

function toFocus(event: { app: string; context?: Record<string, string> }): FocusTarget {
  return event.context === undefined
    ? { app: event.app }
    : { app: event.app, context: event.context };
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
}

export interface FocusTarget {
  app: string;
  context?: Record<string, string>;
}

function sameFocus(a: FocusTarget | undefined, b: FocusTarget | undefined): boolean {
  return a?.app === b?.app && JSON.stringify(a?.context ?? {}) === JSON.stringify(b?.context ?? {});
}

function applySeen(seen: Runtime["seen"], event: SessionEvent): void {
  if (event.type === "app_connected") seen.set(event.app, { v: event.v, tools: event.tools });
}

function applyTold(told: Map<string, number>, event: SessionEvent): void {
  if (event.type === "app_connected") told.set(event.app, event.v);
  else if (event.type === "app_disconnected") told.delete(event.app);
}

export class SessionManager {
  private readonly runtimes = new Map<string, Runtime>();

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
      applyTold(rt.told, record.event);
      applySeen(rt.seen, record.event);
      if (record.event.type === "focus_changed") rt.toldFocus = toFocus(record.event);
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
  }

  /** Cancels the running turn; queued inputs still run. */
  cancel(sessionId: string): void {
    const rt = this.runtime(sessionId);
    if (!rt.running) return;
    if (rt.agentSession) rt.agentSession.cancel();
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
    const told = new Map<string, number>();
    const seen: Runtime["seen"] = new Map();
    for (const r of this.store.readTypes(sessionId, ["app_connected", "app_disconnected"])) {
      applyTold(told, r.event);
      applySeen(seen, r.event);
    }
    const lastFocus = this.store.readTypes(sessionId, ["focus_changed"]).at(-1)?.event;
    const rt: Runtime = {
      queue: [],
      running: false,
      cancelRequested: false,
      pending,
      told,
      seen,
      toldFocus: lastFocus?.type === "focus_changed" ? toFocus(lastFocus) : undefined,
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
    }
  }

  private async runTurn(sessionId: string, rt: Runtime, input: QueuedInput): Promise<void> {
    const ended: TurnEnded = { type: "turn_ended", stopReason: "error" };
    let userLogged = false;
    try {
      const agentSession = rt.agentSession ?? (await this.loadAgentSession(sessionId));
      rt.agentSession = agentSession;
      this.announceConnectors(sessionId, rt);
      const pendingEvents = rt.pending.splice(0);
      this.store.append(sessionId, userMessage(input));
      userLogged = true;
      if (rt.cancelRequested) {
        ended.stopReason = "cancelled";
      } else {
        Object.assign(ended, await this.prompt(sessionId, agentSession, input, pendingEvents));
      }
    } catch (err) {
      // prompt() never rejects by contract, but loading the agent session can fail.
      ended.error = err instanceof Error ? err.message : String(err);
      this.options.log.error(`${sessionId}: turn failed: ${ended.error}`);
      if (!userLogged) this.store.append(sessionId, userMessage(input));
    } finally {
      rt.cancelRequested = false;
    }
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
    const open = new Set(current.map((c) => c.app));
    for (const c of current) {
      if (rt.told.get(c.app) === c.v) continue;
      const event: AppConnected = {
        type: "app_connected",
        app: c.app,
        kind: c.kind,
        v: c.v,
        tools: c.tools,
      };
      const before = rt.seen.get(c.app);
      if (!before) {
        // First time this session hears of it: say what it is for.
        if (c.description !== undefined) event.description = c.description;
      } else if (before.v === c.v) {
        event.reconnect = true;
      } else {
        event.previous = before.v;
        const added = c.tools.filter((t) => !before.tools.includes(t));
        const removed = before.tools.filter((t) => !c.tools.includes(t));
        if (added.length > 0) event.added = added;
        if (removed.length > 0) event.removed = removed;
      }
      this.store.append(sessionId, event);
    }
    for (const app of [...rt.told.keys()]) {
      if (!open.has(app)) this.store.append(sessionId, { type: "app_disconnected", app });
    }
    // <focus> goes last and only when the agent's app or that app's context changed.
    const focus = this.options.focus?.(sessionId);
    if (focus && !sameFocus(focus, rt.toldFocus)) {
      this.store.append(sessionId, { type: "focus_changed", ...focus });
    }
  }

  private async loadAgentSession(sessionId: string): Promise<AgentSession> {
    const row = this.store.getSession(sessionId);
    if (!row) throw new NotFoundError(`no session ${sessionId}`);
    const mcpServers = this.options.mcpServers?.(sessionId) ?? [];
    const params: NewSessionParams = { sessionId, cwd: row.cwd, mcpServers };
    if (this.options.resolveAsset) params.resolveAsset = this.options.resolveAsset;
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
    return info;
  }
}

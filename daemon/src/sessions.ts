// Session manager: one turn at a time per session, inputs queued behind it. Talks to the agent only
// through the ACP-style interface in @suite/harness/agent.
import type { Agent, AgentSession, AgentUpdate, ContextEvent } from "@suite/harness/agent";
import type { SessionEvent, SessionInfo, TurnEnded } from "@suite/protocol/types";
import type { Logger } from "./log.ts";
import type { SessionRow, SessionStore } from "./store.ts";

export class NotFoundError extends Error {}

interface QueuedInput {
  app: string;
  text: string;
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

export function newSessionId(): string {
  const alphabet = "0123456789abcdefghjkmnpqrstvwxyz";
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  return `s_${Array.from(bytes, (b) => alphabet[b % 32]).join("")}`;
}

export interface SessionManagerOptions {
  defaultCwd: string;
  log: Logger;
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
      this.runtimes.get(record.session)?.pending.push(record.event);
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
  input(sessionId: string, app: string, text: string): void {
    const rt = this.runtime(sessionId);
    rt.queue.push({ app, text });
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
    const rt: Runtime = {
      queue: [],
      running: false,
      cancelRequested: false,
      pending,
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
      const pendingEvents = rt.pending.splice(0);
      this.store.append(sessionId, { type: "user_message", app: input.app, text: input.text });
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
      if (!userLogged) {
        this.store.append(sessionId, { type: "user_message", app: input.app, text: input.text });
      }
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

  private async loadAgentSession(sessionId: string): Promise<AgentSession> {
    const row = this.store.getSession(sessionId);
    if (!row) throw new NotFoundError(`no session ${sessionId}`);
    if (row.lastSeq === 0) return this.agent.newSession({ sessionId, cwd: row.cwd });
    const history = this.store.read(sessionId).map((r) => r.event);
    return this.agent.loadSession({ sessionId, cwd: row.cwd, history });
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

// Session manager: one turn at a time per session, inputs queued behind it. Talks to the agent only
// through the ACP-style interface in @neoworks-dev/neo/agent. A session runs on one harness at a time
// (neo, or an ACP harness like Claude Code the user picked with /harness); each has its own agent
// session here, made on first use.
import type {
  Agent,
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  FollowUp,
  ContextEvent,
  McpServer,
  NewSessionParams,
  PromptParams,
  RequestPermission,
  SafeSearch,
} from "@neoworks-dev/neo/agent";
import type {
  SessionEvent,
  SessionInfo,
  TurnEnded,
  WaitingInput,
} from "@neoworks-dev/neo-protocol/types";
import type { ConnectorSnapshot } from "./gateway/registry.ts";
import { handoffSummary } from "./handoff.ts";
import type { Logger } from "./log.ts";
import { MESSAGES_APP } from "./messages.ts";
import type { SessionRow, SessionStore } from "./store.ts";
import { type FocusTarget, type ShowDontTellSetting, ToldState } from "./told.ts";

export type { FocusTarget } from "./told.ts";

export class NotFoundError extends Error {}

/** A request that can't work as asked (the message says why). */
export class BadRequestError extends Error {}

interface QueuedInput {
  app: string;
  text: string;
  images?: string[];
  /** What the chat shows in place of `text` (user_message `shown`). */
  shown?: string;
}

/** In-memory state of a session that has been used since the daemon started. */
interface Runtime {
  queue: QueuedInput[];
  running: boolean;
  /** The agent session of each harness the session has run on, by harness name. */
  agentSessions: Map<string, Promise<AgentSession>>;
  /** Whoever runs the current turn: the agent session, or a guest the user addressed. */
  active?: AgentSession;
  /** Set when cancel() arrives while the agent session is still loading. */
  cancelRequested: boolean;
  /** The running turn was cancelled: nothing more is steered into it. */
  cancelling: boolean;
  /** Inputs are being handed to the running turn (see steer). */
  steering: boolean;
  /** The running turn is a command (/compact…): what is sent meanwhile waits for it to end. */
  command: boolean;
  /** Connector events not yet delivered to the agent. */
  pending: ContextEvent[];
  /** What this session's log has told the agent. */
  told: ToldState;
  /**
   * By app, the inputs it took back out of the queue whose text for the agent is more than the
   * chat shows (see unqueue); its next input brings them back.
   */
  redrafts: Map<string, QueuedInput[]>;
  /** Resolves when the current drain loop finishes. */
  drained: Promise<void>;
}

const CONTEXT_TYPES = new Set<SessionEvent["type"]>([
  "app_connected",
  "app_disconnected",
  "focus_changed",
  "guest_turn",
  "edits_reverted",
  "user_note",
  "show_dont_tell",
]);

function isContextEvent(event: SessionEvent): event is ContextEvent {
  return CONTEXT_TYPES.has(event.type);
}

function userMessage(input: QueuedInput): SessionEvent {
  const event: SessionEvent = { type: "user_message", app: input.app, text: input.text };
  if (input.images) event.images = input.images;
  if (input.shown !== undefined) event.shown = input.shown;
  return event;
}

const isCommand = (input: QueuedInput): boolean => input.text.trimStart().startsWith("/");

/**
 * The next turn's input: what waits at the head of the queue, as one message. Inputs from the
 * same app run together up to a command (a guest's "/claude task", a harness's /command), which
 * gets a turn of its own.
 */
function takeBatch(queue: QueuedInput[]): QueuedInput | undefined {
  const first = queue.shift();
  if (first === undefined || isCommand(first)) return first;
  const batch = [first];
  for (let next = queue[0]; next; next = queue[0]) {
    if (next.app !== first.app || isCommand(next)) break;
    queue.shift();
    batch.push(next);
  }
  return batch.length === 1 ? first : joinInputs(batch);
}

/**
 * Inputs as one, a paragraph each. Each one's `[image N]` markers count its own images, so they
 * are moved past the images of the ones before it.
 */
function joinInputs(inputs: QueuedInput[]): QueuedInput {
  const texts: string[] = [];
  const shown: string[] = [];
  const images: string[] = [];
  for (const input of inputs) {
    const offset = images.length;
    const renumber = (value: string): string =>
      offset === 0
        ? value
        : value.replace(/\[image (\d+)\]/g, (marker, n: string) => {
            const at = Number(n);
            return at >= 1 && at <= (input.images?.length ?? 0) ? `[image ${at + offset}]` : marker;
          });
    texts.push(renumber(input.text));
    shown.push(renumber(input.shown ?? input.text));
    images.push(...(input.images ?? []));
  }
  const joined: QueuedInput = { app: inputs[0]?.app ?? "", text: texts.join("\n\n") };
  if (images.length > 0) joined.images = images;
  if (inputs.some((input) => input.shown !== undefined)) joined.shown = shown.join("\n\n");
  return joined;
}

function waiting(input: QueuedInput): WaitingInput {
  const out: WaitingInput = { app: input.app, text: input.shown ?? input.text };
  if (input.images) out.images = input.images;
  return out;
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
  /** The user's "Show, don't tell." setting; announced as show_dont_tell when it changed. */
  showDontTell?: () => ShowDontTellSetting;
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
  /** The harness the session's turns run on; the agent's own name when absent. */
  harness?: (sessionId: string) => string;
  /** The agent of a harness other than the default one (an ACP harness); undefined: none. */
  agents?: (harness: string) => Agent | undefined;
  /** What the user chose for a harness in this session (model, effort…). */
  settings?: (sessionId: string, harness: string) => Partial<Record<ConfigCategory, string>>;
  /** The hook the agent's own bash, write and edit ask through (see permissions.ts). */
  requestPermission?: (sessionId: string) => RequestPermission;
  /** The user's safe-search choice for the agent's web search (see Settings.safeSearch). */
  safeSearch?: () => SafeSearch | undefined;
  /** Called for each new session (it may start on a harness the user chose before). */
  created?: (sessionId: string) => void;
  /**
   * Takes the messages other agents sent the agent of this session (see messages.ts). They
   * reach a running turn as it goes; what is left when it ends, or arrives while the session is
   * idle, starts a turn.
   */
  takeMessages?: (sessionId: string) => string[];
  /**
   * Whether what the user sends while a turn runs goes into that turn at once, for a harness
   * that can take it (steering), instead of waiting for the next turn. Default: it waits.
   */
  steer?: () => boolean;
  /** A command the session's agent ran ended in the background: tell it (see Messenger.tell). */
  backgroundEnded?: (sessionId: string, text: string) => void;
}

/** Ends the agent sessions a runtime loaded, also those still loading (their SessionEnd hooks run). */
function closeAgentSessions(rt: Runtime): void {
  for (const loading of rt.agentSessions.values()) {
    loading.then(
      (session) => session.close?.("other"),
      () => undefined,
    );
  }
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
  private readonly cancelListeners: ((sessionId: string) => void)[] = [];
  private readonly removeListeners: ((sessionId: string) => void)[] = [];
  private readonly moveListeners: ((sessionId: string) => void)[] = [];

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

  create(params: { cwd?: string; title?: string; parent?: string }): SessionInfo {
    const row = this.store.createSession({
      id: newSessionId(),
      cwd: params.cwd ?? this.options.defaultCwd,
      title: params.title,
      parent: params.parent,
    });
    // A delegated session gets its harness and settings from the delegation, not the user's last.
    if (params.parent === undefined) this.options.created?.(row.id);
    return this.toInfo(row);
  }

  info(id: string): SessionInfo | undefined {
    const row = this.store.getSession(id);
    return row && this.toInfo(row);
  }

  list(): SessionInfo[] {
    return this.store.listSessions().map((row) => this.toInfo(row));
  }

  /**
   * Whether the session can be deleted without losing anything: nobody wrote in it (no
   * user_message), it was not delegated and delegated into nothing (no parent, no children),
   * and nothing runs or waits in it. Whether something still follows it is the caller's to know.
   */
  removable(sessionId: string): boolean {
    const row = this.store.getSession(sessionId);
    if (!row) {
      return false;
    }
    if (row.parent !== undefined || this.store.hasChildren(sessionId)) {
      return false;
    }
    if (this.store.hasEventOfType(sessionId, "user_message")) {
      return false;
    }
    const rt = this.runtimes.get(sessionId);
    if (rt && (rt.running || rt.queue.length > 0)) {
      return false;
    }
    return true;
  }

  /**
   * Moves an idle session to another working directory: the agent sessions it loaded close, so
   * the next turn loads them, with the history, there. onMove listeners are told afterwards (the
   * guests started in the old one stop).
   */
  move(sessionId: string, cwd: string): void {
    if (!this.store.getSession(sessionId)) throw new NotFoundError(`no session ${sessionId}`);
    const rt = this.runtimes.get(sessionId);
    if (rt?.running) throw new BadRequestError("wait until the turn ends, or cancel it");
    this.store.setCwd(sessionId, cwd);
    if (rt) {
      closeAgentSessions(rt);
      rt.agentSessions.clear();
    }
    for (const listener of this.moveListeners) listener(sessionId);
    this.stateChanged(sessionId);
  }

  /**
   * Clears the context of an idle session (/clear): logs context_cleared and closes the agent
   * sessions it loaded, so the next turn starts a new conversation on each harness, told about
   * the open apps afresh. The log, todos and settings stay.
   */
  clearContext(sessionId: string): void {
    if (!this.store.getSession(sessionId)) throw new NotFoundError(`no session ${sessionId}`);
    const rt = this.runtime(sessionId);
    if (rt.running) throw new BadRequestError("wait until the turn ends, or cancel it");
    closeAgentSessions(rt);
    rt.agentSessions.clear();
    rt.pending = [];
    const event: SessionEvent = { type: "context_cleared" };
    rt.told.apply(event);
    this.store.append(sessionId, event);
    this.stateChanged(sessionId);
  }

  /**
   * Deletes a session and its log, and closes the agent sessions it loaded. onRemove listeners
   * are told afterwards, to drop what they keep for it. Check removable() first.
   */
  remove(sessionId: string): void {
    const rt = this.runtimes.get(sessionId);
    this.runtimes.delete(sessionId);
    this.store.deleteSession(sessionId);
    if (rt) {
      closeAgentSessions(rt);
    }
    this.options.log.info(`removed empty session ${sessionId}`);
    for (const listener of this.removeListeners) listener(sessionId);
  }

  /** Deletes every removable session; for daemon start, when nothing follows any yet. */
  pruneEmpty(): void {
    for (const row of this.store.listSessions()) {
      if (this.removable(row.id)) {
        this.remove(row.id);
      }
    }
  }

  /**
   * Queues input. When this returns, info() already shows the session running or queued. `shown`
   * is what the chat shows when `text` is written for the agent.
   */
  input(sessionId: string, app: string, text: string, images?: string[], shown?: string): void {
    const rt = this.runtime(sessionId);
    const input: QueuedInput = images?.length ? { app, text, images } : { app, text };
    if (shown !== undefined) input.shown = shown;
    // What the app took back out of the queue and sends again unchanged gets its text for the
    // agent back (a neoui press's context); one that was edited goes as written.
    const redrafts = rt.redrafts.get(app);
    rt.redrafts.delete(app);
    for (const redraft of redrafts ?? []) {
      if (redraft.shown === undefined || !input.text.includes(redraft.shown)) continue;
      input.shown ??= text;
      input.text = input.text.replace(redraft.shown, () => redraft.text);
    }
    rt.queue.push(input);
    this.kick(sessionId, rt);
    this.stateChanged(sessionId);
    // A harness that can be steered gets it in the running turn at once, if the user wants that.
    if (rt.running && this.options.steer?.() === true) void this.steer(sessionId, rt);
  }

  /**
   * Sends a command the running turn waits for to the background (see AgentSession.background);
   * false when there is none or its harness can't.
   */
  async background(sessionId: string, toolCallId: string): Promise<boolean> {
    const active = this.runtimes.get(sessionId)?.active;
    if (!active?.background) return false;
    return (await active.background(toolCallId)) === true;
  }

  /** Starts working off the queue, unless a turn already runs. */
  private kick(sessionId: string, rt: Runtime): void {
    if (rt.running) return;
    rt.running = true;
    rt.drained = this.drain(sessionId, rt);
  }

  /**
   * Hands the inputs at the head of the queue, up to a command, to the running turn of a harness
   * that can be steered (ACP), one by one, each logged as user_message once it took it. One it
   * doesn't take (the turn just ended) goes back to the head of the queue and waits for a turn.
   */
  private async steer(sessionId: string, rt: Runtime): Promise<void> {
    if (rt.steering) return;
    rt.steering = true;
    try {
      for (let input = rt.queue[0]; input; input = rt.queue[0]) {
        const active = rt.active;
        if (isCommand(input) || !active?.steer || rt.cancelling || rt.command) return;
        rt.queue.shift();
        const taken = await active
          .steer(input.images ? { text: input.text, images: input.images } : { text: input.text })
          .catch(() => false);
        if (!taken) {
          rt.queue.unshift(input);
          this.kick(sessionId, rt);
          return;
        }
        this.store.append(sessionId, userMessage(input));
        this.stateChanged(sessionId);
      }
    } finally {
      rt.steering = false;
    }
  }

  /**
   * Takes the inputs `app` queued behind the running turn back out of the queue, oldest first,
   * as the user wrote them (to edit them again). Throws NotFoundError for an unknown session.
   * Those whose text for the agent says more are kept, so the app's next input can bring it back.
   */
  unqueue(sessionId: string, app: string): WaitingInput[] {
    const rt = this.runtime(sessionId);
    const taken = rt.queue.filter((input) => input.app === app);
    if (taken.length === 0) return [];
    rt.queue = rt.queue.filter((input) => input.app !== app);
    const hidden = taken.filter((input) => input.shown !== undefined && input.shown !== input.text);
    rt.redrafts.set(app, hidden);
    this.stateChanged(sessionId);
    return taken.map(waiting);
  }

  /** The harness the session's next turn runs on. */
  harness(sessionId: string): string {
    return this.options.harness?.(sessionId) ?? this.agent.name;
  }

  /**
   * A harness's settable options for this session (default: the one it runs on); loads its
   * agent session if needed.
   */
  async configOptions(
    sessionId: string,
    harness = this.harness(sessionId),
  ): Promise<ConfigOption[]> {
    const agentSession = await this.agentSession(sessionId, harness);
    if (agentSession.loadConfigOptions) return agentSession.loadConfigOptions();
    return agentSession.configOptions?.() ?? [];
  }

  /** Sets one for the harness's following turns; rejects if it doesn't take it. */
  async configure(
    sessionId: string,
    category: ConfigCategory,
    value: string,
    harness = this.harness(sessionId),
  ): Promise<void> {
    const agentSession = await this.agentSession(sessionId, harness);
    if (!agentSession.setConfigOption) {
      throw new BadRequestError(`${harness} has no settings`);
    }
    try {
      await agentSession.setConfigOption(category, value);
    } catch (err) {
      throw new BadRequestError(err instanceof Error ? err.message : String(err));
    }
  }

  /**
   * The second step a harness asks for after a setting was set (neo: which OpenRouter host);
   * undefined when it has none.
   */
  async followUp(
    sessionId: string,
    category: ConfigCategory,
    value: string,
    harness = this.harness(sessionId),
  ): Promise<FollowUp | undefined> {
    const agentSession = await this.agentSession(sessionId, harness);
    return agentSession.followUp?.(category, value);
  }

  private agentSession(sessionId: string, harness: string): Promise<AgentSession> {
    const rt = this.runtime(sessionId);
    let session = rt.agentSessions.get(harness);
    if (!session) {
      const loading = this.loadAgentSession(sessionId, harness);
      session = loading;
      rt.agentSessions.set(harness, loading);
      // A failed start is tried again next time.
      loading.catch(() => {
        if (rt.agentSessions.get(harness) === loading) rt.agentSessions.delete(harness);
      });
    }
    return session;
  }

  /** Starts a turn on the messages waiting for the session's agent, when it is idle. */
  wake(sessionId: string): void {
    if (!this.store.getSession(sessionId) || this.runtime(sessionId).running) return;
    const input = this.messageInput(sessionId);
    if (input) this.input(sessionId, input.app, input.text);
  }

  /** The turn the messages waiting for the session's agent make, if there are any. */
  private messageInput(sessionId: string): QueuedInput | undefined {
    const messages = this.options.takeMessages?.(sessionId) ?? [];
    return messages.length > 0 ? { app: MESSAGES_APP, text: messages.join("\n\n") } : undefined;
  }

  /** Cancels the running turn; queued inputs still run. */
  cancel(sessionId: string): void {
    for (const listener of this.cancelListeners) listener(sessionId);
    const rt = this.runtime(sessionId);
    if (!rt.running) return;
    rt.cancelling = true;
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
    // The sessions the agent has loaded are over: its SessionEnd hooks run.
    const closes: Promise<void>[] = [];
    for (const rt of this.runtimes.values()) {
      for (const loading of rt.agentSessions.values()) {
        closes.push(
          loading.then(
            (session) => session.close?.("other"),
            () => undefined,
          ),
        );
      }
    }
    await Promise.race([Promise.all(closes), Bun.sleep(timeoutMs)]);
  }

  /**
   * After a crash the log can end inside a turn. Close those turns so views and the next prompt
   * see a consistent log; the agent repairs its own context on load. Questions left open are
   * dropped too: nothing waits for their answer any more, and a pane would offer them forever.
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
      // A subagent in the background outlives turns, but not the daemon.
      const open = new Set<string>();
      for (const { event } of this.store.readTypes(row.id, [
        "subagent_started",
        "subagent_ended",
      ])) {
        if (event.type === "subagent_started" && event.background === true) open.add(event.id);
        else if (event.type === "subagent_ended") open.delete(event.id);
      }
      for (const id of open) {
        this.store.append(row.id, {
          type: "subagent_ended",
          id,
          stopReason: "error",
          error: "Interrupted: the daemon stopped.",
        });
      }
      // Answered with no option, as Asker.dropAll does on a clean stop.
      const asked = new Map<string, "choice_made" | "permission_resolved">();
      for (const { event } of this.store.readTypes(row.id, [
        "choice",
        "choice_made",
        "permission_request",
        "permission_resolved",
      ])) {
        if (event.type === "choice") asked.set(event.id, "choice_made");
        else if (event.type === "permission_request") asked.set(event.id, "permission_resolved");
        else if (event.type === "choice_made" || event.type === "permission_resolved") {
          asked.delete(event.id);
        }
      }
      for (const [id, type] of asked) this.store.append(row.id, { type, id });
      if (asked.size > 0)
        this.options.log.info(`dropped ${asked.size} open questions in ${row.id}`);
    }
  }

  private runtime(sessionId: string): Runtime {
    const existing = this.runtimes.get(sessionId);
    if (existing) return existing;
    if (!this.store.getSession(sessionId)) throw new NotFoundError(`no session ${sessionId}`);
    // Connector events since the last prompt (or /clear) are still pending after a restart.
    const lastInput = Math.max(
      this.store.lastSeqOfType(sessionId, "user_message"),
      this.store.lastSeqOfType(sessionId, "context_cleared"),
    );
    const pending = this.store
      .read(sessionId, lastInput + 1)
      .map((r) => r.event)
      .filter(isContextEvent);
    const told = new ToldState();
    const types: SessionEvent["type"][] = [
      "app_connected",
      "app_disconnected",
      "focus_changed",
      "show_dont_tell",
      "context_cleared",
    ];
    for (const r of this.store.readTypes(sessionId, types)) told.apply(r.event);
    const rt: Runtime = {
      queue: [],
      running: false,
      agentSessions: new Map(),
      cancelRequested: false,
      cancelling: false,
      steering: false,
      command: false,
      pending,
      told,
      redrafts: new Map(),
      drained: Promise.resolve(),
    };
    this.runtimes.set(sessionId, rt);
    return rt;
  }

  private async drain(sessionId: string, rt: Runtime): Promise<void> {
    try {
      let input = takeBatch(rt.queue);
      while (input) {
        // What it took out of the queue is no longer waiting.
        this.stateChanged(sessionId);
        const stopReason = await this.runTurn(sessionId, rt, input);
        // A turn that ended well goes on with what other agents wrote meanwhile.
        input =
          takeBatch(rt.queue) ??
          (stopReason === "end_turn" ? this.messageInput(sessionId) : undefined);
      }
    } finally {
      rt.running = false;
      this.stateChanged(sessionId);
    }
  }

  private async runTurn(
    sessionId: string,
    rt: Runtime,
    input: QueuedInput,
  ): Promise<TurnEnded["stopReason"]> {
    const command = guestCommand(input.text);
    const guest = command && this.options.guest?.(sessionId, command.name);
    if (command && guest) {
      return this.runGuestTurn(sessionId, rt, input, guest, command.task);
    }
    const ended: TurnEnded = { type: "turn_ended", stopReason: "error" };
    let userLogged = false;
    try {
      let harness = this.harness(sessionId);
      let agentSession = await this.agentSession(sessionId, harness);
      // A /harness while it loaded: the turn runs on the one the user chose last.
      for (let now = this.harness(sessionId); now !== harness; now = this.harness(sessionId)) {
        harness = now;
        agentSession = await this.agentSession(sessionId, harness);
      }
      this.announceConnectors(sessionId, rt);
      const pendingEvents = rt.pending.splice(0);
      // What the turns another harness ran said, for one that wasn't there.
      const summary = handoffSummary(this.store, sessionId, harness, this.agent.name);
      this.store.append(sessionId, userMessage(input));
      userLogged = true;
      if (rt.cancelRequested) {
        ended.stopReason = "cancelled";
      } else {
        rt.active = agentSession;
        rt.command = isCommand(input);
        // The user's next inputs join this turn as it goes, while it runs on the same harness,
        // when the user lets them (see steer); else they wait for the next turn.
        const takeInputs = (): QueuedInput[] => {
          if (this.options.steer?.() !== true) return [];
          if (rt.command || this.harness(sessionId) !== harness) return [];
          return this.takeInputs(sessionId, rt);
        };
        const turn = await this.prompt(
          sessionId,
          agentSession,
          input,
          pendingEvents,
          summary,
          takeInputs,
        );
        Object.assign(ended, turn);
      }
    } catch (err) {
      // prompt() never rejects by contract, but loading the agent session can fail.
      ended.error = err instanceof Error ? err.message : String(err);
      this.options.log.error(`${sessionId}: turn failed: ${ended.error}`);
      if (!userLogged) this.store.append(sessionId, userMessage(input));
    } finally {
      rt.cancelRequested = false;
      rt.cancelling = false;
      rt.command = false;
      rt.active = undefined;
    }
    this.store.append(sessionId, ended);
    return ended.stopReason;
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
  ): Promise<TurnEnded["stopReason"]> {
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
    rt.cancelling = false;
    this.store.append(sessionId, ended);
    return ended.stopReason;
  }

  private async prompt(
    sessionId: string,
    agentSession: AgentSession,
    input: QueuedInput,
    pendingEvents: ContextEvent[],
    summary?: string,
    takeInputs?: () => QueuedInput[],
  ): Promise<Omit<TurnEnded, "type">> {
    let open = true;
    const onUpdate = (update: AgentUpdate): void => {
      if (open) this.store.append(sessionId, update);
      else this.options.log.warn(`${sessionId}: dropped ${update.type} sent after the turn ended`);
    };
    try {
      const params: PromptParams = { ...input, pendingEvents, onUpdate };
      if (summary !== undefined) params.summary = summary;
      const { takeMessages } = this.options;
      if (takeMessages) params.takeMessages = () => takeMessages(sessionId);
      if (takeInputs) {
        params.takeInputs = () =>
          takeInputs().map(({ text, images }) => (images ? { text, images } : { text }));
      }
      const result = await agentSession.prompt(params);
      const out: Omit<TurnEnded, "type"> = { stopReason: result.stopReason };
      if (result.error !== undefined) out.error = result.error;
      return out;
    } finally {
      open = false;
    }
  }

  /**
   * Takes the inputs at the head of the queue a running turn can be told: not a command (a guest's
   * "/claude task", a harness's own /command), which waits for a turn of its own, nor anything
   * behind it. Each is logged as the user_message it is now.
   */
  private takeInputs(sessionId: string, rt: Runtime): QueuedInput[] {
    const taken: QueuedInput[] = [];
    for (let input = rt.queue[0]; input; input = rt.queue[0]) {
      if (isCommand(input)) break;
      rt.queue.shift();
      this.store.append(sessionId, userMessage(input));
      taken.push(input);
    }
    if (taken.length > 0) this.stateChanged(sessionId);
    return taken;
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
    const focus = this.options.focus?.(sessionId);
    for (const event of rt.told.changes(current, focus, this.options.showDontTell?.())) {
      this.store.append(sessionId, event);
    }
  }

  private async loadAgentSession(sessionId: string, harness: string): Promise<AgentSession> {
    const row = this.store.getSession(sessionId);
    if (!row) throw new NotFoundError(`no session ${sessionId}`);
    const isDefault = harness === this.agent.name;
    const agent = isDefault ? this.agent : this.options.agents?.(harness);
    if (!agent) throw new BadRequestError(`no harness named ${harness}`);
    const mcpServers = this.options.mcpServers?.(sessionId) ?? [];
    const params: NewSessionParams = { sessionId, cwd: row.cwd, mcpServers };
    if (this.options.resolveAsset) params.resolveAsset = this.options.resolveAsset;
    // An ACP harness asks through its adapter's requests instead.
    if (isDefault && this.options.requestPermission) {
      params.requestPermission = this.options.requestPermission(sessionId);
    }
    if (isDefault && this.options.safeSearch) params.safeSearch = this.options.safeSearch;
    const { backgroundEnded } = this.options;
    if (isDefault && backgroundEnded) {
      params.backgroundEnded = (text) => backgroundEnded(sessionId, text);
    }
    const settings = this.options.settings?.(sessionId, harness);
    if (settings && Object.keys(settings).length > 0) params.settings = settings;
    if (row.lastSeq === 0) return agent.newSession(params);
    const history = this.store.read(sessionId).map((r) => r.event);
    return agent.loadSession({ ...params, history });
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
    if (rt && rt.queue.length > 0) info.waiting = rt.queue.map(waiting);
    if (row.title !== undefined) info.title = row.title;
    if (row.parent !== undefined) info.parent = row.parent;
    if (this.options.describe) Object.assign(info, this.options.describe(row.id));
    return info;
  }

  /** Called when someone cancels a session's turn (also when it is idle). */
  onCancel(listener: (sessionId: string) => void): void {
    this.cancelListeners.push(listener);
  }

  /** Called when a session was deleted (see remove). */
  onRemove(listener: (sessionId: string) => void): void {
    this.removeListeners.push(listener);
  }

  /** Called when a session moved to another working directory (see move). */
  onMove(listener: (sessionId: string) => void): void {
    this.moveListeners.push(listener);
  }

  /** Called when a session starts or stops running, or its queue changes. */
  onStateChange(listener: (sessionId: string) => void): void {
    this.stateListeners.push(listener);
  }

  /** Something in the session's info changed that no event says, such as its title. */
  changed(sessionId: string): void {
    this.stateChanged(sessionId);
  }

  private stateChanged(sessionId: string): void {
    for (const listener of this.stateListeners) listener(sessionId);
  }
}

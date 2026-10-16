// Where the agent lives: the current session (the one every app follows) and the app whose pane
// hosts it. The daemon owns this; apps only ask to move it (agent.move) and follow agent.focus.
import type { AgentFocus, FocusReason } from "@suite/protocol/types";
import type { ConnectorRegistry } from "./gateway/registry.ts";
import type { Logger } from "./log.ts";
import { NotFoundError, type SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";

export class BadRequestError extends Error {}

const CURRENT_SESSION = "current_session";

export class FocusManager {
  private session: string;
  private app: string | undefined;
  private last: AgentFocus;
  private readonly listeners = new Set<(focus: AgentFocus) => void>();

  constructor(
    private readonly store: SessionStore,
    private readonly sessions: SessionManager,
    private readonly registry: ConnectorRegistry,
    private readonly log: Logger,
  ) {
    const saved = store.getMeta(CURRENT_SESSION);
    const known = saved !== undefined && sessions.info(saved) !== undefined;
    this.session = known ? saved : (sessions.list()[0]?.id ?? sessions.create({}).id);
    store.setMeta(CURRENT_SESSION, this.session);
    this.app = registry.home();
    this.last = { session: this.session, reason: "initial" };
    if (this.app !== undefined) this.last.app = this.app;
    registry.onChange(() => this.followConnectors());
  }

  /** The current state, as sent to a client that just connected. */
  current(): AgentFocus {
    const focus: AgentFocus = { session: this.session, reason: "initial" };
    if (this.app !== undefined) focus.app = this.app;
    return focus;
  }

  /** The app hosting the agent and its context, if `session` is the current one. */
  focusOf(session: string): { app: string; context?: Record<string, string> } | undefined {
    if (session !== this.session || this.app === undefined) return undefined;
    const context = this.registry.presenceOf(this.app)?.context;
    return context === undefined ? { app: this.app } : { app: this.app, context };
  }

  /**
   * Moves the agent. No app means home (or nowhere if the home app isn't open); a session makes
   * that session current. Throws NotFoundError / BadRequestError for impossible moves.
   */
  move(target: { app?: string; session?: string }, reason: FocusReason): AgentFocus {
    if (target.session !== undefined && !this.sessions.info(target.session)) {
      throw new NotFoundError(`no session ${target.session}`);
    }
    if (target.app !== undefined) {
      const presence = this.registry.presenceOf(target.app);
      if (!presence) throw new NotFoundError(`${target.app} isn't open`);
      if (presence.kind !== "app")
        throw new BadRequestError(`${target.app} is a service, not an app`);
    }
    const session = target.session ?? this.session;
    // A user switching sessions (e.g. the overlay's reset) is reported as such.
    const effective = session !== this.session && reason === "user" ? "session" : reason;
    return this.set(session, target.app ?? this.registry.home(), effective);
  }

  /**
   * Input typed into an open app's pane moves the agent there (and makes that session current),
   * so "this" means what the user is looking at. Input from anything else (the CLI) doesn't.
   */
  followInput(session: string, app: string): void {
    if (app === this.app && session === this.session) return;
    if (this.registry.presenceOf(app)?.kind !== "app") return;
    if (!this.sessions.info(session)) return;
    this.set(session, app, "input");
  }

  onChange(listener: (focus: AgentFocus) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Falls back home when the agent's app closes; the home app picks it up when it opens. */
  private followConnectors(): void {
    if (this.app !== undefined && !this.registry.isOpen(this.app)) {
      this.set(this.session, this.registry.home(), "closed");
    } else if (this.app === undefined) {
      const home = this.registry.home();
      if (home !== undefined) this.set(this.session, home, "opened");
    }
  }

  private set(session: string, app: string | undefined, reason: FocusReason): AgentFocus {
    const changed = session !== this.session || app !== this.app;
    if (!changed) return this.last;
    const focus: AgentFocus = { session, reason };
    if (app !== undefined) focus.app = app;
    if (this.app !== undefined && this.app !== app) focus.from = this.app;
    if (session !== this.session) this.store.setMeta(CURRENT_SESSION, session);
    this.session = session;
    this.app = app;
    this.last = focus;
    this.log.info(`agent → ${session} in ${app ?? "no app"} (${reason})`);
    for (const listener of this.listeners) listener(focus);
    return focus;
  }
}

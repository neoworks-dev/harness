// One live SessionView per session (and per agent, for a per-agent view) that someone has looked
// at, kept current from the event log.
import type { AppLibrary, CallTool, Program } from "@neoworks/neoui";
import type { ViewPatch } from "@neoworks-dev/neo-protocol/types";
import type { Logger } from "../log.ts";
import type { SessionStore } from "../store.ts";
import { SessionView, type TemplateLookup } from "./session-view.ts";

export class ViewManager {
  private readonly views = new Map<string, SessionView>();
  private readonly listeners = new Set<(patch: ViewPatch) => void>();

  constructor(
    private readonly store: SessionStore,
    private readonly templates: TemplateLookup,
    private readonly log: Logger,
    /** Runs the tools a neoui program's Query and Mutation name, for a session. */
    private readonly tools?: (session: string) => CallTool,
    /** The library with the components of the apps, for the programs of every view. */
    private readonly apps?: () => AppLibrary,
  ) {
    store.onAppend((record) => {
      for (const [key, view] of this.views) {
        const { session, agent } = parseViewKey(key);
        if (session !== record.session) continue;
        const ops = view.apply(record);
        if (ops.length === 0) continue;
        const patch: ViewPatch = { session, seq: record.seq, ops };
        if (agent !== undefined) patch.agent = agent;
        for (const listener of this.listeners) listener(patch);
      }
    });
  }

  /** The session's view, built from its log the first time; with `agent`, that agent's own. */
  get(session: string, agent?: string): SessionView {
    const key = viewKey(session, agent);
    const existing = this.views.get(key);
    if (existing) return existing;
    const view = new SessionView(
      session,
      this.templates,
      this.log,
      agent,
      this.tools?.(session),
      this.apps,
    );
    view.replay(this.store.read(session));
    // Changes outside the event log (a neoui doc answering) go out as patches of their own.
    view.onOps = (ops) => {
      const patch: ViewPatch = { session, seq: view.lastSeq, ops };
      if (agent !== undefined) patch.agent = agent;
      for (const listener of this.listeners) listener(patch);
    };
    this.views.set(key, view);
    return view;
  }

  /**
   * The running neoui doc `doc` of item `ref` in the session, and the view that has it (an
   * item of the session's own agent is in the session's view, a guest's in its own).
   */
  openUi(
    session: string,
    ref: string,
    doc: string,
  ): { program: Program; view: SessionView } | undefined {
    const main = this.get(session);
    const candidates = [main];
    for (const [key, view] of this.views) {
      if (view !== main && parseViewKey(key).session === session) candidates.push(view);
    }
    for (const view of candidates) {
      const program = view.program(ref, doc);
      if (program) return { program, view };
    }
    return undefined;
  }

  /** Drops the views of a session that was deleted. */
  forget(session: string): void {
    for (const key of [...this.views.keys()]) {
      if (parseViewKey(key).session === session) {
        this.views.delete(key);
      }
    }
  }

  onPatch(listener: (patch: ViewPatch) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

/** What a client's subscription is keyed by: the session, and the agent for a per-agent view. */
export function viewKey(session: string, agent?: string): string {
  return agent === undefined ? session : `${session}\0${agent}`;
}

/** The session and agent a viewKey was made of. */
export function parseViewKey(key: string): { session: string; agent?: string } {
  const at = key.indexOf("\0");
  return at < 0 ? { session: key } : { session: key.slice(0, at), agent: key.slice(at + 1) };
}

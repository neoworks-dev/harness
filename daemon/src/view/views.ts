// One live SessionView per session (and per agent, for a per-agent view) that someone has looked
// at, kept current from the event log.
import type { ViewPatch } from "@suite/protocol/types";
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
  ) {
    store.onAppend((record) => {
      for (const [key, view] of this.views) {
        const { session, agent } = parseKey(key);
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
    let view = this.views.get(key);
    if (!view) {
      view = new SessionView(session, this.templates, this.log, agent);
      for (const record of this.store.read(session)) view.apply(record);
      this.views.set(key, view);
    }
    return view;
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

function parseKey(key: string): { session: string; agent?: string } {
  const at = key.indexOf("\0");
  return at < 0 ? { session: key } : { session: key.slice(0, at), agent: key.slice(at + 1) };
}

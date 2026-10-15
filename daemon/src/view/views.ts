// One live SessionView per session that someone has looked at, kept current from the event log.
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
      const view = this.views.get(record.session);
      if (!view) return;
      const ops = view.apply(record);
      if (ops.length === 0) return;
      const patch: ViewPatch = { session: record.session, seq: record.seq, ops };
      for (const listener of this.listeners) listener(patch);
    });
  }

  /** The session's view, built from its log the first time. */
  get(session: string): SessionView {
    let view = this.views.get(session);
    if (!view) {
      view = new SessionView(session, this.templates, this.log);
      for (const record of this.store.read(session)) view.apply(record);
      this.views.set(session, view);
    }
    return view;
  }

  onPatch(listener: (patch: ViewPatch) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

// What an agent has been told about connectors and focus, and the events that bring it up to date.
// The session's own agent learns it from the session log; each guest keeps its own copy.
import type { ContextEvent } from "@neoworks-dev/neo/agent";
import type { AppConnected, SessionEvent } from "@neoworks-dev/neo-protocol/types";
import type { ConnectorSnapshot } from "./gateway/registry.ts";

export interface FocusTarget {
  app: string;
  context?: Record<string, string>;
}

export function toFocus(event: { app: string; context?: Record<string, string> }): FocusTarget {
  return event.context === undefined
    ? { app: event.app }
    : { app: event.app, context: event.context };
}

function sameFocus(a: FocusTarget | undefined, b: FocusTarget | undefined): boolean {
  return a?.app === b?.app && JSON.stringify(a?.context ?? {}) === JSON.stringify(b?.context ?? {});
}

/** The user's "Show, don't tell." setting, and the UI library to give the agent when it is on. */
export interface ShowDontTellSetting {
  on: boolean;
  library: () => string;
}

export class ToldState {
  /** What the agent thinks is open: app → version it was told about. */
  private readonly told = new Map<string, number>();
  /** Every connector it ever heard of (open or not): the last version, tools and components. */
  private readonly seen = new Map<string, { v: number; tools: string[]; components: string[] }>();
  /** The last focus it was told. */
  private focus?: FocusTarget;
  /** Whether it was last asked to show, not tell (never asked: it wasn't). */
  private showDontTell = false;

  /** Records an event the agent was (or will be) told. */
  apply(event: SessionEvent): void {
    if (event.type === "show_dont_tell") {
      this.showDontTell = event.on;
    } else if (event.type === "app_connected") {
      this.told.set(event.app, event.v);
      // An event that does not say keeps what it knew.
      let components = event.components;
      if (components === undefined) components = this.seen.get(event.app)?.components;
      components ??= [];
      this.seen.set(event.app, { v: event.v, tools: event.tools, components });
    } else if (event.type === "app_disconnected") {
      this.told.delete(event.app);
    } else if (event.type === "focus_changed") {
      this.focus = toFocus(event);
    }
  }

  /**
   * show_dont_tell when the user's setting isn't what the agent was last told (with the UI
   * library when it is on), app_connected / app_disconnected for whatever changed since the
   * agent was last told (by version), then focus_changed if its app or that app's context
   * changed. Doesn't apply them.
   */
  changes(
    current: ConnectorSnapshot[],
    focus: FocusTarget | undefined,
    showDontTell?: ShowDontTellSetting,
  ): ContextEvent[] {
    const events: ContextEvent[] = [];
    if (showDontTell && showDontTell.on !== this.showDontTell) {
      events.push(
        showDontTell.on
          ? { type: "show_dont_tell", on: true, library: showDontTell.library() }
          : { type: "show_dont_tell", on: false },
      );
    }
    const open = new Set(current.map((c) => c.app));
    for (const c of current) {
      if (this.told.get(c.app) === c.v) continue;
      const event: AppConnected = {
        type: "app_connected",
        app: c.app,
        kind: c.kind,
        v: c.v,
        tools: c.tools,
      };
      const before = this.seen.get(c.app);
      const components = c.components ?? [];
      if (!before) {
        // First time it hears of it: say what it is for, and what it adds to the UI library.
        if (c.description !== undefined) event.description = c.description;
        if (components.length > 0) event.components = components;
      } else if (before.v === c.v) {
        event.reconnect = true;
      } else {
        event.previous = before.v;
        if (before.components.join("\0") !== components.join("\0")) {
          event.components = components;
        }
        const added = c.tools.filter((t) => !before.tools.includes(t));
        const removed = before.tools.filter((t) => !c.tools.includes(t));
        if (added.length > 0) event.added = added;
        if (removed.length > 0) event.removed = removed;
      }
      events.push(event);
    }
    for (const app of this.told.keys()) {
      if (!open.has(app)) events.push({ type: "app_disconnected", app });
    }
    if (focus && !sameFocus(focus, this.focus)) events.push({ type: "focus_changed", ...focus });
    return events;
  }
}

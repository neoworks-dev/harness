// What the agents list holds: the agents the shown session's events say are working (or just
// finished), and whether the list has the keyboard.
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { applyAgentEvent, type AgentRow } from "./agents.ts";

const NONE: readonly AgentRow[] = [];

class Agents {
  /** Ctrl-X: the list has the keyboard (arrows select, Enter opens, x stops, Esc gives it back). */
  focused = $state(false);
  private readonly fold = new Folded<readonly AgentRow[]>(NONE, (rows, record) =>
    applyAgentEvent(rows, record),
  );

  /** Every agent of the shown session, in the order they started. */
  get rows(): readonly AgentRow[] {
    return this.fold.value;
  }

  /** Folds the session's events until the plugin unloads. */
  follow(daemon: DaemonService): void {
    this.fold.follow(daemon);
  }
}

export const agents = new Agents();

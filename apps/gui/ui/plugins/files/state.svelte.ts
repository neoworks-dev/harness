// The files the shown session's agents touched, from its events, and a count of the moments git
// is worth asking again: a file was touched, or a turn ended.
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { NO_TOUCHES, applyTouch, type Touches } from "./files.ts";

interface Folding {
  touches: Touches;
  /** Bumped when a turn ends or a file is touched. */
  changes: number;
}

const NONE: Folding = { touches: NO_TOUCHES, changes: 0 };

class Touched {
  private readonly fold = new Folded<Folding>(NONE, (state, record, daemon) => {
    const touches = applyTouch(state.touches, record, daemon.cwd);
    const changed = touches.touched !== state.touches.touched || record.event.type === "turn_ended";
    return { touches, changes: changed ? state.changes + 1 : state.changes };
  });

  get touches(): Touches {
    return this.fold.value.touches;
  }

  get changes(): number {
    return this.fold.value.changes;
  }

  /** Folds the session's events until the plugin unloads. */
  follow(daemon: DaemonService): void {
    this.fold.follow(daemon);
  }
}

export const touched = new Touched();

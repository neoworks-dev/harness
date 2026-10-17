// The timeline of the shown session (timeline.ts), folded from its events and replayed with them
// when a session is opened.
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { applyRecord, emptyTimeline, type Timeline } from "./timeline.ts";

const EMPTY = emptyTimeline();

class Timelines {
  // The maps are changed in place, as a session has thousands of events and the fold must not copy
  // them each time; the new object around them is what tells the readers.
  private readonly fold = new Folded<Timeline>(EMPTY, (state, record) => {
    const timeline = state === EMPTY ? emptyTimeline() : state;
    return { ...applyRecord(timeline, record) };
  });

  /** What the events of the session shown now say; empty for any other. */
  get value(): Timeline {
    return this.fold.value;
  }

  /** Folds the session's events until the plugin unloads. */
  follow(daemon: DaemonService): void {
    this.fold.follow(daemon);
  }
}

export const timeline = new Timelines();

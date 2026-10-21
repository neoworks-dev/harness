// State folded from the shown session's events: a pane that shows what they say beyond the chat
// (the agents working, the plan, the files touched) keeps one. The daemon replays a session's
// history when it is opened, so a fold begins again when the events are of another session or
// start over; and it reads as `initial` while the session shown is another one.
import type { SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import type { DaemonService } from "./services/daemon.ts";

type Reduce<S> = (state: S, record: SessionEventRecord, daemon: DaemonService) => S;

export class Folded<S> {
  /** The session the folded state belongs to. */
  private session = $state<string>();
  private lastSeq = 0;
  private folded = $state.raw<S>();
  private daemon?: DaemonService;

  constructor(
    private readonly initial: S,
    private readonly reduce: Reduce<S>,
  ) {
    this.folded = initial;
  }

  /** What the events of the session shown now say; `initial` for any other. */
  get value(): S {
    const current = this.session !== undefined && this.session === this.daemon?.sessionId;
    return current && this.folded !== undefined ? this.folded : this.initial;
  }

  /** Folds every event from now on, until the plugin that called unloads. */
  follow(daemon: DaemonService): void {
    this.daemon = daemon;
    daemon.onRecord((record) => this.fold(record, daemon));
  }

  private fold(record: SessionEventRecord, daemon: DaemonService): void {
    // A new session, or the same one replayed from the start: begin again.
    if (record.session !== this.session || record.seq <= this.lastSeq) {
      this.session = record.session;
      this.folded = this.initial;
    }
    this.lastSeq = record.seq;
    this.folded = this.reduce(this.folded ?? this.initial, record, daemon);
  }
}

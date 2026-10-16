// What the shown session's events say beyond the chat: the plan, the agents working for it and
// the files they touched. Folded as the events arrive (the daemon replays a session's history
// when it is opened), and forgotten when another session is shown.
import { applyAgentEvent, type AgentRow } from "../lib/agents.ts";
import { NO_TOUCHES, applyTouch, type Touches } from "../lib/files.ts";
import { applyPlan, type PlanList } from "../lib/plan.ts";
import { suite } from "../state/suite.svelte.ts";

/** What there is to show when the events are of another session: nothing. */
const NONE: readonly never[] = [];

class Activity {
  /** The session the folded state belongs to. */
  private session = $state<string>();
  private lastSeq = 0;
  private _plans = $state.raw<readonly PlanList[]>([]);
  private _agents = $state.raw<readonly AgentRow[]>([]);
  private _touches = $state.raw<Touches>(NO_TOUCHES);
  /** Bumped when a turn ends or a file is edited: git is worth asking again. */
  private _changes = $state(0);

  constructor() {
    suite.onRecord((record) => this.fold(record));
  }

  /** Only what belongs to the session shown now. */
  private get current(): boolean {
    return this.session !== undefined && this.session === suite.sessionId;
  }

  get plans(): readonly PlanList[] {
    if (!this.current) return NONE;
    return this._plans;
  }

  get agents(): readonly AgentRow[] {
    if (!this.current) return NONE;
    return this._agents;
  }

  get touches(): Touches {
    if (!this.current) return NO_TOUCHES;
    return this._touches;
  }

  get changes(): number {
    return this._changes;
  }

  private fold(record: Parameters<typeof applyAgentEvent>[1]): void {
    // A new session, or the same one replayed from the start: begin again.
    if (record.session !== this.session || record.seq <= this.lastSeq) {
      this.session = record.session;
      this._plans = [];
      this._agents = [];
      this._touches = NO_TOUCHES;
    }
    this.lastSeq = record.seq;
    const { event } = record;
    if (event.type === "plan") this._plans = applyPlan(this._plans, event);
    this._agents = applyAgentEvent(this._agents, record);
    const touches = applyTouch(this._touches, record, suite.cwd);
    if (touches.touched !== this._touches.touched || event.type === "turn_ended")
      this._changes += 1;
    this._touches = touches;
  }
}

export const activity = new Activity();

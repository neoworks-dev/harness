// The plans the shown session's events say: the agent's todo list, and each guest's.
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { applyPlan, type PlanList } from "./plan.ts";

const NONE: readonly PlanList[] = [];

class Plans {
  private readonly fold = new Folded<readonly PlanList[]>(NONE, (lists, { event }) =>
    event.type === "plan" ? applyPlan(lists, event) : lists,
  );

  /** One list per agent that has a plan. */
  get lists(): readonly PlanList[] {
    return this.fold.value;
  }

  /** Folds the session's events until the plugin unloads. */
  follow(daemon: DaemonService): void {
    this.fold.follow(daemon);
  }
}

export const plans = new Plans();

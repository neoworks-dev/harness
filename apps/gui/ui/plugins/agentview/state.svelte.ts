// The one agent whose own chat is shown in place of the session's (Enter in the agents list). The
// view has the keyboard while it is open, so the input does not take it back until it is closed.
import type { ViewItem } from "@suite/protocol/types";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";

/** What the view's title needs, kept from when it was opened: the row may be gone by then. */
export interface ViewedAgent {
  harness: string;
  /** As it is shown. */
  model?: string | undefined;
  task: string;
}

class AgentView {
  agent = $state.raw<ViewedAgent>();
  items = $state.raw<readonly ViewItem[]>([]);
  private session = $state<string>();
  private daemon = $state.raw<DaemonService>();
  private stop: (() => void) | undefined;

  /** Follows the session shown until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    this.daemon = daemon;
    const stopEffects = $effect.root(() => {
      // Another session is shown: that agent was this one's.
      $effect(() => {
        if (this.agent !== undefined && this.session !== daemon.sessionId)
          untrack(() => this.close());
      });
    });
    return () => {
      stopEffects();
      this.close();
      this.daemon = undefined;
    };
  }

  get isOpen(): boolean {
    return this.agent !== undefined && this.session === this.daemon?.sessionId;
  }

  /** The agent's title line: harness, model and the first line of its task. */
  get title(): { harness: string; model?: string; task: string } | undefined {
    const agent = this.agent;
    if (agent === undefined) return undefined;
    const task = agent.task.split("\n").find((line) => line.trim() !== "") ?? "";
    const title: { harness: string; model?: string; task: string } = {
      harness: agent.harness,
      task,
    };
    if (agent.model !== undefined) title.model = agent.model;
    return title;
  }

  open(agent: ViewedAgent): void {
    const daemon = this.daemon;
    if (daemon === undefined) return;
    this.close();
    this.session = daemon.sessionId;
    this.agent = agent;
    this.items = [];
    // The daemon resolves the view of just that agent's work in the session.
    this.stop = daemon.followAgent(agent.harness, (items) => (this.items = items));
  }

  close(): void {
    if (this.agent === undefined) return;
    this.stop?.();
    this.stop = undefined;
    this.agent = undefined;
    this.items = [];
  }
}

export const agentView = new AgentView();

// The one agent whose own chat is shown in place of the session's (Enter in the agents list).
// It keeps the Ctrl-X mode on while it is open, so the input does not take the keyboard back
// until the view is closed.
import type { ViewItem } from "@suite/protocol/types";
import { untrack } from "svelte";
import { modelLabel, type AgentRow } from "../lib/agents.ts";
import { suite } from "../state/suite.svelte.ts";
import { ui } from "../state/ui.svelte.ts";

/** What the view's title needs, kept from when it was opened: the row may be gone by then. */
export interface ViewedAgent {
  id: string;
  harness: string;
  model?: string;
  task: string;
}

class AgentView {
  agent = $state.raw<ViewedAgent>();
  items = $state.raw<readonly ViewItem[]>([]);
  private session = $state<string>();
  private stop: (() => void) | undefined;

  constructor() {
    // Another session is shown: that agent was this one's.
    $effect.root(() => {
      $effect(() => {
        if (this.agent !== undefined && this.session !== suite.sessionId)
          untrack(() => this.close());
      });
    });
  }

  get isOpen(): boolean {
    return this.agent !== undefined && this.session === suite.sessionId;
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
    if (agent.model !== undefined) title.model = modelLabel(agent.model);
    return title;
  }

  open(row: AgentRow): void {
    this.close();
    this.session = suite.sessionId;
    const agent: ViewedAgent = { id: row.id, harness: row.harness, task: row.task };
    if (row.model !== undefined) agent.model = row.model;
    this.agent = agent;
    this.items = [];
    // The daemon resolves the view of just that agent's work in the session.
    this.stop = suite.followAgent(row.harness, (items) => (this.items = items));
    ui.agentsFocused = true;
  }

  close(): void {
    if (this.agent === undefined) return;
    this.stop?.();
    this.stop = undefined;
    this.agent = undefined;
    this.items = [];
    ui.agentsFocused = false;
  }
}

export const agentView = new AgentView();

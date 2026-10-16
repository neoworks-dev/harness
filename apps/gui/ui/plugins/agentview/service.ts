// The `agentview` service: what a pane that lists agents needs to show one agent's own chat.
import { Service, type Context } from "@neoworks/extension-system";
import { agentView, type ViewedAgent } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    agentview: AgentViewService;
  }
}

export class AgentViewService extends Service {
  constructor(ctx: Context) {
    super(ctx, "agentview");
  }

  /** Whether an agent's chat is shown in place of the session's. */
  get isOpen(): boolean {
    return agentView.isOpen;
  }

  /** Shows `agent`'s chat of the session shown now, until it is closed or another session shows. */
  open(agent: ViewedAgent): void {
    agentView.open(agent);
  }
}

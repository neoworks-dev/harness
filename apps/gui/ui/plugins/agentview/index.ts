// One agent's own chat in place of the session's, read-only, drawn like the main chat (through
// `chat.item`). It has the keyboard while it is open. The agents list opens it through the
// `agentview` service.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import AgentView from "./AgentView.svelte";
import { AgentViewService } from "./service.ts";
import { agentView } from "./state.svelte.ts";

export const agentViewPane = {
  name: "agentview",
  inject: ["slots", "daemon", "keyboard", "transcript"],
  apply(ctx: Context) {
    const { slots, daemon, keyboard } = ctx;
    ctx.effect(() => agentView.start(daemon), "agent view state");
    ctx.plugin(AgentViewService);

    // The middle shows an agent's chat while one is open, ahead of the session's own.
    slots.inject("shell.main", (slot) => {
      slot.add({ component: AgentView, order: 20, select: () => agentView.isOpen });
    });
    keyboard.claim(() => agentView.isOpen);
  },
};

// The panes from before they were plugins, each mounted into its slot as it was drawn then. A
// pane that is converted moves into a plugin of its own (plugins/sessions is the example) and
// leaves this file; the shell's slots are all it needs to know about.
import type { Context } from "@neoworks/extension-system";
import AgentView from "../../side/AgentView.svelte";
import { agentView } from "../../side/agentview.svelte.ts";
import Agents from "../../side/Agents.svelte";
import GitHub from "../../side/GitHub.svelte";
import SidePanels from "../../side/SidePanels.svelte";
import { ui } from "../../state/ui.svelte.ts";
import "../shell/slots.ts";

// Several draw a diff, so they need the service that shows it over the window.
const inject = ["slots", "diff", "transcript"];

// The middle shows the GitHub view when it is open, else an agent's own chat, else the chat.
const github = {
  name: "legacy/github",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.main", (slot) => {
      slot.add({ component: GitHub, order: 10, select: () => ui.githubOpen });
    }),
};

const agentChat = {
  name: "legacy/agent-view",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.main", (slot) => {
      slot.add({ component: AgentView, order: 20, select: () => agentView.isOpen });
    }),
};

const agents = {
  name: "legacy/agents",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: Agents, order: 20 });
    }),
};

const sidePanels = {
  name: "legacy/side-panels",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.right", (slot) => {
      slot.add({ component: SidePanels });
    }),
};

export const panes = [github, agentChat, agents, sidePanels];

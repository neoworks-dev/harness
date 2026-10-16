// The panes from before they were plugins, each mounted into its slot as it was drawn then. A
// pane that is converted moves into a plugin of its own (plugins/sessions is the example) and
// leaves this file; the shell's slots are all it needs to know about.
import type { Context } from "@neoworks/extension-system";
import Chat from "../../chat/Chat.svelte";
import DiffOverlay from "../../chat/DiffOverlay.svelte";
import Composer from "../../input/Composer.svelte";
import StatusLine from "../../input/StatusLine.svelte";
import AgentView from "../../side/AgentView.svelte";
import { agentView } from "../../side/agentview.svelte.ts";
import Agents from "../../side/Agents.svelte";
import GitHub from "../../side/GitHub.svelte";
import Sessions from "../../side/Sessions.svelte";
import SidePanels from "../../side/SidePanels.svelte";
import { ui } from "../../state/ui.svelte.ts";
import "../shell/slots.ts";

const inject = ["slots"];

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

const chat = {
  name: "legacy/chat",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.main", (slot) => {
      slot.add({ component: Chat, order: 100, select: () => true });
    }),
};

const composer = {
  name: "legacy/composer",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: Composer, order: 10 });
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

const statusLine = {
  name: "legacy/status-line",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: StatusLine, order: 30 });
    }),
};

const sessions = {
  name: "legacy/sessions",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.left", (slot) => {
      slot.add({ component: Sessions });
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

const diffOverlay = {
  name: "legacy/diff-overlay",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.overlay", (slot) => {
      slot.add({ component: DiffOverlay });
    }),
};

export const panes = [
  sessions,
  github,
  agentChat,
  chat,
  composer,
  agents,
  statusLine,
  sidePanels,
  diffOverlay,
];

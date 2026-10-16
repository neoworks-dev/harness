// Every plugin of the window, in the order they mount (so the order of their registrations in a
// list slot where `order` ties, and of the `/` menu). Plugins are compiled into the bundle.
import type { Plugin } from "@neoworks/extension-system";
import { agentsList } from "./agents/index.ts";
import { agentViewPane } from "./agentview/index.ts";
import { composerPane } from "./composer/index.ts";
import { panes } from "./legacy/panes.ts";
import { shortcuts } from "./legacy/shortcuts.ts";
import { diffOverlay } from "./diff/index.ts";
import { sessionsSidebar } from "./sessions/index.ts";
import { shell } from "./shell/index.ts";
import { statusLine } from "./statusline/index.ts";
import { transcriptPane } from "./transcript/index.ts";

export const plugins: Plugin[] = [
  shell,
  sessionsSidebar,
  statusLine,
  transcriptPane,
  shortcuts,
  composerPane,
  agentViewPane,
  agentsList,
  diffOverlay,
  ...panes,
];

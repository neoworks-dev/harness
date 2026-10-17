// Every plugin of the window, in the order they mount (so the order of their registrations in a
// list slot where `order` ties, and of the `/` menu). Plugins are compiled into the bundle. A
// plugin that provides a service (viewer, comments, transcript, agentview, sidepanel) comes before those that
// use it; one that needs a service that is not mounted yet waits for it.
import type { Plugin } from "@neoworks/extension-system";
import { agentsList } from "./agents/index.ts";
import { agentViewPane } from "./agentview/index.ts";
import { commentsStore } from "./comments/index.ts";
import { composerPane } from "./composer/index.ts";
import { filesPanel } from "./files/index.ts";
import { githubPane } from "./github/index.ts";
import { planPanel } from "./plan/index.ts";
import { sessionsSidebar } from "./sessions/index.ts";
import { shell } from "./shell/index.ts";
import { sidePanel } from "./sidepanel/index.ts";
import { statusLine } from "./statusline/index.ts";
import { transcriptPane } from "./transcript/index.ts";
import { workspaceCard } from "./workspace/index.ts";
import { viewerPane } from "./viewer/index.ts";

export const plugins: Plugin[] = [
  shell,
  sessionsSidebar,
  statusLine,
  commentsStore,
  viewerPane,
  transcriptPane,
  githubPane,
  composerPane,
  agentViewPane,
  agentsList,
  sidePanel,
  planPanel,
  filesPanel,
  workspaceCard,
];

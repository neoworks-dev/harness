// Every plugin of the window, in the order they mount (so the order of their registrations in a
// list slot where `order` ties, and of the `/` menu). Plugins are compiled into the bundle.
import type { Plugin } from "@neoworks/extension-system";
import { panes } from "./legacy/panes.ts";
import { shortcuts } from "./legacy/shortcuts.ts";
import { sessionsSidebar } from "./sessions/index.ts";
import { shell } from "./shell/index.ts";
import { statusLine } from "./statusline/index.ts";

export const plugins: Plugin[] = [shell, sessionsSidebar, statusLine, shortcuts, ...panes];

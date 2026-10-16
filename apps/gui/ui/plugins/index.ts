// Every plugin of the window, in the order they mount (so the order of their registrations in a
// list slot where `order` ties, and of the `/` menu). Plugins are compiled into the bundle.
import type { Plugin } from "@neoworks/extension-system";
import { shortcuts } from "./legacy/shortcuts.ts";

export const plugins: Plugin[] = [shortcuts];

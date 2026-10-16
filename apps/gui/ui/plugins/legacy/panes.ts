// The panes from before they were plugins, each mounted into its slot as it was drawn then. A
// pane that is converted moves into a plugin of its own (plugins/sessions is the example) and
// leaves this file; the shell's slots are all it needs to know about.
import type { Context } from "@neoworks/extension-system";
import GitHub from "../../side/GitHub.svelte";
import { ui } from "../../state/ui.svelte.ts";
import "../shell/slots.ts";

// Several draw a diff, so they need the service that shows it over the window.
const inject = ["slots", "diff"];

// The middle shows the GitHub view while it is open.
const github = {
  name: "legacy/github",
  inject,
  apply: ({ slots }: Context) =>
    slots.inject("shell.main", (slot) => {
      slot.add({ component: GitHub, order: 10, select: () => ui.githubOpen });
    }),
};

export const panes = [github];

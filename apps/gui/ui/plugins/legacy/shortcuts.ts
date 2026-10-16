// The keys and the local `/` command of the panes that are not plugins yet. They reach into the
// panes' stores directly; each moves into the plugin of the pane it belongs to when that pane is
// converted (see plugins/composer for the input's keys and plugins/transcript for Ctrl-O and Ctrl-T).
//
// The keys reach the window when the focused input or panel does not use them. A handler that
// returns false has not used the key.
import type { Context } from "@neoworks/extension-system";
import { ui } from "../../state/ui.svelte.ts";

export const shortcuts = {
  name: "legacy/shortcuts",
  inject: ["keymap", "keyboard", "commands"],
  apply({ keymap, keyboard, commands }: Context) {
    // The GitHub view has the keyboard while it is open.
    keyboard.claim(() => ui.githubOpen);

    keymap.bind("ctrl-r", () => void (ui.githubOpen = !ui.githubOpen));

    commands.register({
      name: "github",
      desc: "GitHub issues and pull requests (ctrl-r)",
      run: () => (ui.githubOpen = !ui.githubOpen),
    });
  },
};

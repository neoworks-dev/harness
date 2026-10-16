// The window's keys and local `/` commands from before panes were plugins. They reach into the
// panes' stores directly; each moves into the plugin of the pane it belongs to when that pane is
// converted (see the sessions plugin for Ctrl-S, F6 and Ctrl-N, and its `/sidebar`).
//
// The keys reach the window when the focused input or panel does not use them. A handler that
// returns false has not used the key.
import { window } from "@neoworks/svelte-gpui";
import type { Context } from "@neoworks/extension-system";
import { composer } from "../../input/composer.svelte.ts";
import { chord } from "../../kernel/keymap.ts";
import { suite } from "../../state/suite.svelte.ts";
import { ui } from "../../state/ui.svelte.ts";

export const shortcuts = {
  name: "legacy/shortcuts",
  inject: ["keymap", "commands"],
  apply({ keymap, commands }: Context) {
    // Ctrl-C is left to the input too, so copying a selection still works; any other key
    // disarms a first Ctrl-C.
    keymap.observe((event) => {
      if (!/^ctrl-(shift-)?c$/.test(chord(event) ?? "")) composer.otherKey();
    });
    for (const copy of ["ctrl-c", "ctrl-shift-c"]) {
      keymap.bind(copy, () => {
        composer.ctrlC();
        return false;
      });
    }

    keymap.bind("shift-tab", () => void suite.cycleApprovals());
    keymap.bind("escape", (event) => {
      // Something that took Escape itself (a popover, the diff overlay) has already closed.
      if (event.defaultPrevented) return false;
      if (composer.escape()) return true;
      if (!suite.status.running) return false;
      void suite.cancel();
    });
    keymap.bind("ctrl-x", () => void (ui.agentsFocused = !ui.agentsFocused));
    keymap.bind("ctrl-r", () => void (ui.githubOpen = !ui.githubOpen));
    keymap.bind("ctrl-p", () => composer.reopenPrompts());

    commands.register({
      name: "github",
      desc: "GitHub issues and pull requests (ctrl-r)",
      run: () => (ui.githubOpen = !ui.githubOpen),
    });
    commands.register({
      name: "help",
      desc: "keys and commands",
      run: () => (composer.helpOpen = true),
    });
    commands.register({
      name: "quit",
      desc: "close the window; the session keeps running",
      run: () => window.close(),
    });
  },
};

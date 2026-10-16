// The GitHub view (Ctrl-R, `/github`), in place of the chat: the issues and pull requests of the
// repository `gh` finds for the session's directory. It takes the middle ahead of the
// conversation (`shell.main` is a chain) and the keyboard while it is shown. Read-only.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import GitHub from "./GitHub.svelte";
import { github } from "./state.svelte.ts";

export const githubPane = {
  name: "github",
  inject: ["slots", "daemon", "github", "diff", "keymap", "keyboard", "commands"],
  apply(ctx: Context) {
    const { slots, daemon, github: host, diff, keymap, keyboard, commands } = ctx;
    ctx.effect(() => github.start({ daemon, host, diff }), "github state");

    slots.inject("shell.main", (slot) => {
      slot.add({ component: GitHub, order: 10, select: () => github.shown });
    });
    keyboard.claim(() => github.shown);

    const toggle = (): void => {
      github.shown = !github.shown;
    };
    keymap.bind("ctrl-r", toggle);
    commands.register({
      name: "github",
      desc: "GitHub issues and pull requests (ctrl-r)",
      run: toggle,
    });
  },
};

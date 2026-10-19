// A secret the daemon asks this window for (`secret.request`): a masked field above the input.
// The question is not a card: it is not in the chat or the session log, and the typed text goes
// to the daemon and nowhere else.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import Secret from "./Secret.svelte";
import { secretField } from "./state.svelte.ts";

export const secretPane = {
  name: "secret",
  inject: ["slots", "daemon", "keyboard"],
  apply(ctx: Context) {
    const { slots, daemon, keyboard } = ctx;
    ctx.effect(() => secretField.start(daemon), "secret question");

    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: Secret, order: 5 });
    });

    // The input stays away from the keyboard while the question is open.
    keyboard.claim(() => secretField.ask !== undefined);
  },
};

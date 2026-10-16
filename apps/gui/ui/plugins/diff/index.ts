// The side-by-side diff over the window, and the `diff` service that opens it. Esc closes it,
// ahead of whatever else Esc closes, since it covers everything.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import DiffOverlay from "./DiffOverlay.svelte";
import { DiffService } from "./service.ts";
import { viewing } from "./state.svelte.ts";

export const diffOverlay = {
  name: "diff",
  inject: ["slots", "daemon", "keymap"],
  apply(ctx: Context) {
    const { slots, keymap } = ctx;
    ctx.plugin(DiffService);

    slots.inject("shell.overlay", (slot) => {
      slot.add({ component: DiffOverlay });
    });

    keymap.bind("escape", () => {
      if (viewing.current === undefined) return false;
      viewing.current = undefined;
    });
  },
};

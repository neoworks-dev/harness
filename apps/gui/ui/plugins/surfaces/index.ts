// Surfaces (docs/neoui.md): neoui programs of the session that stay, drawn in their place rather
// than in the chat. `bottom` ones over the input, `side` ones as cards of the side panel, a
// `page` in the chat's place (Esc, or the bar over it, goes back; a chip over the input returns).
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import "../sidepanel/slots.ts";
import BottomSurfaces from "./BottomSurfaces.svelte";
import PagePane from "./PagePane.svelte";
import SideSurfaces from "./SideSurfaces.svelte";
import { surfaces } from "./state.svelte.ts";
import { inPlace } from "./surfaces.ts";

export const surfacesPane = {
  name: "surfaces",
  inject: ["slots", "daemon", "keymap", "sidepanel"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, sidepanel } = ctx;
    ctx.effect(() => surfaces.start(daemon), "surfaces state");

    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: BottomSurfaces, order: 8 });
    });
    slots.inject("sidepanel.section", (slot) => {
      slot.add({ component: SideSurfaces, order: 5 });
    });
    sidepanel.claim(() => inPlace(daemon.surfaces, "side").length > 0);
    slots.inject("shell.main", (slot) => {
      slot.add({ component: PagePane, order: 3, select: () => surfaces.page !== undefined });
    });

    keymap.bind("escape", () => {
      if (surfaces.page === undefined) return false;
      surfaces.page = undefined;
    });
  },
};

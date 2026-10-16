// The column at the right, shown while a session is shown, the window is wide enough and a
// section has something to show. Sections (the plan, the files) register into
// `sidepanel.section` and claim the column through the `sidepanel` service.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import { SidePanelService } from "./service.ts";
import SidePanel from "./SidePanel.svelte";
import "./slots.ts";

export const sidePanel = {
  name: "sidepanel",
  inject: ["slots", "daemon"],
  apply(ctx: Context) {
    const { slots } = ctx;
    ctx.plugin(SidePanelService);

    slots.inject("shell.right", (slot) => {
      slot.add({ component: SidePanel, slots: { "sidepanel.section": "list" } });
    });
  },
};

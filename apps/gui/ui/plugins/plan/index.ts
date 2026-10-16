// The plan, in the column at the right: the todo list of the agent and of each guest that has one.
import type { Context } from "@neoworks/extension-system";
import "../sidepanel/slots.ts";
import Plan from "./Plan.svelte";
import { plans } from "./state.svelte.ts";

export const planPanel = {
  name: "plan",
  inject: ["slots", "daemon", "sidepanel"],
  apply(ctx: Context) {
    const { slots, daemon, sidepanel } = ctx;
    plans.follow(daemon);

    slots.inject("sidepanel.section", (slot) => {
      slot.add({ component: Plan, order: 10 });
    });
    sidepanel.claim(() => plans.lists.length > 0);
  },
};

// The `sidepanel` service: how a section tells the column at the right that it has something to
// show, since the column is only worth its width then.
import { Service, type Context } from "@neoworks/extension-system";
import { sections } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    sidepanel: SidePanelService;
  }
}

export class SidePanelService extends Service {
  constructor(ctx: Context) {
    super(ctx, "sidepanel");
  }

  /** The section has something to show while `has` says so (it may read reactive state). */
  claim(has: () => boolean): () => void {
    const dispose = this.ctx.effect(() => sections.add(has), "sidepanel.claim");
    return () => void dispose();
  }
}

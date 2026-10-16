// The `transcript` service: what another pane that draws chat items, or answers one, needs of it.
import { Service, type Context } from "@neoworks/extension-system";
import type { ViewItem } from "@suite/protocol/types";
import { transcript } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    transcript: TranscriptService;
  }
}

export class TranscriptService extends Service {
  constructor(ctx: Context) {
    super(ctx, "transcript");
  }

  /** The items to draw (Ctrl-T shows the thoughts too); reactive. */
  visible(items: readonly ViewItem[]): ViewItem[] {
    return transcript.visible(items);
  }

  /** Draws a one-line stub in place of item `ref`, as it is answered elsewhere; returns the call that ends it. */
  stub(ref: string): () => void {
    return transcript.stub(ref);
  }
}

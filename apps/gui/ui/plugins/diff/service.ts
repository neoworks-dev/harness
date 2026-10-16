// The `diff` service: how another pane shows a diff over the window, side by side.
import { Service, type Context } from "@neoworks/extension-system";
import type { DiffComponent } from "@suite/protocol/types";
import { viewing } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    diff: DiffService;
  }
}

export class DiffService extends Service {
  constructor(ctx: Context) {
    super(ctx, "diff");
  }

  /** Shows `diff` over the window until it is closed (Esc, or its button). */
  show(diff: DiffComponent): void {
    viewing.current = diff;
  }

  close(): void {
    viewing.current = undefined;
  }
}

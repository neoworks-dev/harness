// The `viewer` service: how another pane shows a diff or a file in the panel beside the chat.
import { Service, type Context } from "@neoworks/extension-system";
import type { DiffComponent } from "@neoworks-dev/neo-protocol/types";
import type { FileDiff } from "../../lib/unified.ts";
import { viewer } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    viewer: ViewerService;
  }
}

export class ViewerService extends Service {
  constructor(ctx: Context) {
    super(ctx, "viewer");
  }

  /** Whether the panel is open. */
  get isOpen(): boolean {
    return viewer.shown;
  }

  /** Opens the diff of the session's uncommitted changes, with `path` brought into view. */
  showChanges(path?: string): void {
    viewer.showChanges(path);
  }

  /** Shows a diff the agent put in the chat (one file). */
  showDiff(diff: DiffComponent): void {
    viewer.showDiff(diff);
  }

  /** Shows a patch of several files under a title, such as a pull request's diff. */
  showPatch(title: string, files: readonly FileDiff[]): void {
    viewer.showPatch(title, files);
  }

  /** Opens a file of the session's directory in a tab. */
  openFile(path: string): void {
    viewer.openFile(path);
  }

  toggle(): void {
    viewer.toggle();
  }

  close(): void {
    viewer.close();
  }
}

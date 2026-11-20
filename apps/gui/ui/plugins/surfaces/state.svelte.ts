// The page surface shown in the chat's place, if any. A page that opens is shown at once; Esc or
// the bar over it goes back to the chat, and a chip over the input brings it back.
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { pageAfter } from "./surfaces.ts";

class Surfaces {
  /** The ref of the page shown in the chat's place; undefined shows the chat. */
  page = $state<string>();
  private seen = new Set<string>();
  private session: string | undefined;

  /** Follows the shown session's surfaces until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    return $effect.root(() => {
      $effect(() => {
        const surfaces = daemon.surfaces;
        const session = daemon.sessionId;
        untrack(() => {
          // Another session: what it had open before was opened then, not now.
          if (session !== this.session) {
            this.session = session;
            this.seen = new Set(surfaces.map((surface) => surface.ref));
            this.page = undefined;
            return;
          }
          this.page = pageAfter(this.page, surfaces, this.seen);
          this.seen = new Set(surfaces.map((surface) => surface.ref));
        });
      });
    });
  }
}

export const surfaces = new Surfaces();

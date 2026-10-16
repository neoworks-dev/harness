// Which sessions finished a turn while another one was shown and have not been looked at since:
// the sidebar marks them with a green dot. Kept for the life of the window, not of the sidebar,
// so a session that finishes while the sidebar is closed is still marked when it opens.
import type { SessionInfo } from "@suite/protocol/types";
import { untrack } from "svelte";
import { finishedAfter } from "../lib/sessions.ts";
import { suite } from "../state/suite.svelte.ts";

class Finished {
  ids = $state.raw<ReadonlySet<string>>(new Set());
  private previous: readonly SessionInfo[] = [];

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const next = suite.sessions;
        const shown = suite.sessionId;
        untrack(() => {
          this.ids = finishedAfter(this.ids, this.previous, next, shown);
          this.previous = next;
        });
      });
    });
  }
}

export const finished = new Finished();

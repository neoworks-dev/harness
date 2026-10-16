// Where each session's directory belongs (repository and worktree). The `.worktrees/<name>`
// convention is read in the UI; linked worktrees elsewhere need the host to look at the `.git`
// files, which it does once per directory.
import * as host from "../kernel/host/git.ts";
import type { Place } from "../lib/sessions.ts";

class Places {
  /** What the host found, by directory. */
  found = $state.raw<Readonly<Record<string, Place>>>({});
  private asked = new Set<string>();

  /** Asks for the directories not asked about yet. A failure leaves them to the convention. */
  ensure(cwds: readonly string[]): void {
    const fresh = [...new Set(cwds)].filter((cwd) => !this.asked.has(cwd));
    if (fresh.length === 0) return;
    for (const cwd of fresh) this.asked.add(cwd);
    host.places(fresh).then(
      (places) => (this.found = { ...this.found, ...places }),
      () => {},
    );
  }
}

export const places = new Places();

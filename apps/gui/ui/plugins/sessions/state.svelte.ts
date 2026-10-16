// What the sessions sidebar holds beyond what the daemon says: whether it is shown and has the
// keyboard, which sessions finished unseen, and where each session's directory belongs.
import type { SessionInfo } from "@suite/protocol/types";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import * as host from "../../kernel/host/git.ts";
import { finishedAfter, type Place } from "./sessions.ts";

class Sessions {
  /** Ctrl-S: the sidebar is shown. */
  open = $state(true);
  /** F6: the sidebar has the keyboard (arrows move, Enter opens, Esc gives it back). */
  focused = $state(false);
  /**
   * Which sessions finished a turn while another one was shown and have not been looked at since:
   * marked with a green dot. Kept while the plugin is mounted, not while the sidebar is shown, so
   * a session that finishes with the sidebar closed is still marked when it opens.
   */
  finished = $state.raw<ReadonlySet<string>>(new Set());
  /**
   * Where each session's directory belongs (repository and worktree). The `.worktrees/<name>`
   * convention is read in the UI; linked worktrees elsewhere need the host to look at the `.git`
   * files, which it does once per directory.
   */
  places = $state.raw<Readonly<Record<string, Place>>>({});

  private previous: readonly SessionInfo[] = [];
  private asked = new Set<string>();

  /** Follows the sessions the daemon lists until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    return $effect.root(() => {
      $effect(() => {
        const next = daemon.sessions;
        const shown = daemon.sessionId;
        untrack(() => {
          this.finished = finishedAfter(this.finished, this.previous, next, shown);
          this.previous = next;
        });
      });
      $effect(() => {
        const cwds = daemon.sessions.map((session) => session.cwd);
        untrack(() => this.ask(cwds));
      });
    });
  }

  /** Asks for the directories not asked about yet. A failure leaves them to the convention. */
  private ask(cwds: readonly string[]): void {
    const fresh = [...new Set(cwds)].filter((cwd) => !this.asked.has(cwd));
    if (fresh.length === 0) return;
    for (const cwd of fresh) this.asked.add(cwd);
    host.places(fresh).then(
      (places) => (this.places = { ...this.places, ...places }),
      () => {},
    );
  }
}

export const sessions = new Sessions();

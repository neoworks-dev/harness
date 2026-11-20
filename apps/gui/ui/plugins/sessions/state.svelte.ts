// What the sessions sidebar holds beyond what the daemon says: whether it is shown and has the
// keyboard, which sessions finished unseen, where each session's directory belongs, what was
// searched for and which project is shown, and the user's pins and settled sessions.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import * as host from "../../kernel/host/git.ts";
import { target } from "../../kernel/target.ts";
import { loadMarks, saveMarks } from "./marks-store.ts";
import { mergeMarks, NO_MARKS, pin, toggleSettled, unpin, type Marks } from "./marks.ts";
import { finishedAfter, runningSinceAfter, type Place } from "./sessions.ts";

class Sessions {
  /** Ctrl-S: the sidebar is shown. */
  /** Shown; a column beside another session's starts without it (Ctrl-S shows it). */
  open = $state(!target.column);
  /** F6: the sidebar has the keyboard (arrows move, Enter opens, Esc gives it back). */
  focused = $state(false);
  /**
   * Which sessions finished a turn while another one was shown and have not been looked at since:
   * marked with a green dot. Kept while the plugin is mounted, not while the sidebar is shown, so
   * a session that finishes with the sidebar closed is still marked when it opens.
   */
  finished = $state.raw<ReadonlySet<string>>(new Set());
  /** When each running session was first seen running (Unix ms). */
  runningSince = $state.raw<Readonly<Record<string, number>>>({});
  /**
   * Where each session's directory belongs (repository and worktree). The `.worktrees/<name>`
   * convention is read in the UI; linked worktrees elsewhere need the host to look at the `.git`
   * files, which it does once per directory.
   */
  places = $state.raw<Readonly<Record<string, Place>>>({});
  /** The branch checked out in each session's directory; null outside a repository. */
  branches = $state.raw<Readonly<Record<string, string | null>>>({});

  /** What the search field holds. */
  query = $state("");
  /** Bumped to ask the search field for the keyboard (Ctrl-Shift-F). */
  searchRequest = $state(0);
  /** The repository the list is scoped to; undefined shows every project. Kept while the app runs. */
  project = $state<string>();

  /** The pinned and settled sessions, kept on disk. */
  marks = $state.raw<Marks>(NO_MARKS);
  /** "Settled" is unfolded, and shows all its sessions and not the first ten. */
  settledOpen = $state(false);
  settledAll = $state(false);
  /** The repositories and worktrees whose older sessions are shown. */
  groupsOpen = $state.raw<ReadonlySet<string>>(new Set());

  /** "N older" of a group: its older sessions too, or (pressed again on its heading) not. */
  toggleGroup(group: string): void {
    const next = new Set(this.groupsOpen);
    if (!next.delete(group)) next.add(group);
    this.groupsOpen = next;
  }

  private previous: readonly SessionInfo[] = [];
  private asked = new Set<string>();
  /** The saved marks have been read. Writing waits for it, so a quick change is not undone by it. */
  private marksRead: Promise<void> = Promise.resolve();

  /** Follows the sessions the daemon lists until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    this.marksRead = loadMarks().then((saved) => {
      // What was marked while the file was read wins over what it says.
      this.marks = mergeMarks(saved, this.marks);
    });
    return $effect.root(() => {
      $effect(() => {
        const next = daemon.sessions;
        const shown = daemon.sessionId;
        untrack(() => {
          this.finished = finishedAfter(this.finished, this.previous, next, shown);
          this.runningSince = runningSinceAfter(this.runningSince, next, Date.now());
          this.previous = next;
        });
      });
      $effect(() => {
        const cwds = daemon.sessions.map((session) => session.cwd);
        untrack(() => this.ask(cwds));
      });
    });
  }

  /** Asks the sidebar to focus the search field, showing the sidebar first. */
  focusSearch(): void {
    this.open = true;
    this.searchRequest += 1;
  }

  togglePin(info: SessionInfo, pinned: boolean): void {
    this.mark(pinned ? unpin(this.marks, info.id) : pin(this.marks, info.id, Date.now()));
  }

  /** Settles a session and its subagents' sessions, or brings them back. */
  toggleSettle(info: SessionInfo, settled: boolean): void {
    this.mark(toggleSettled(this.marks, info, this.previous, settled));
  }

  private mark(next: Marks): void {
    this.marks = next;
    void this.marksRead.then(() => saveMarks(this.marks));
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
    for (const cwd of fresh) {
      host.branch(cwd).then(
        (branch) => (this.branches = { ...this.branches, [cwd]: branch }),
        () => {},
      );
    }
  }
}

export const sessions = new Sessions();

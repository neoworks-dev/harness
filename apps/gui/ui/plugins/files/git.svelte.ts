// The git status of the shown session's directory, for the files panel: asked for when the
// session changes, when its events say files were edited or a turn ended (a moment later, so a
// burst of edits is one ask), and every few seconds anyway because the user edits too.
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import type { GitService } from "../../kernel/services/git.ts";
import { parseStatus, type GitStatus } from "../../lib/git-status.ts";

const DEBOUNCE_MS = 200;
const REFRESH_MS = 10_000;

interface Services {
  daemon: DaemonService;
  git: GitService;
}

class Git {
  /** Absent outside a repository, and before the first answer. */
  status = $state.raw<GitStatus>();
  private services?: Services;
  private asking = false;
  private again = false;

  /** Keeps the status current until the returned call stops it; `changes` says when to ask. */
  start(services: Services, changes: () => number): () => void {
    const { daemon } = services;
    this.services = services;
    const stop = $effect.root(() => {
      // The directory or the events changed: ask soon.
      $effect(() => {
        const cwd = daemon.cwd;
        void changes();
        if (cwd === "") return;
        const timer = setTimeout(() => void this.refresh(), DEBOUNCE_MS);
        return () => clearTimeout(timer);
      });
      $effect(() => {
        if (daemon.cwd === "") return;
        const timer = setInterval(() => void this.refresh(), REFRESH_MS);
        return () => clearInterval(timer);
      });
      // Another directory: the old answer is not about it.
      $effect(() => {
        void daemon.cwd;
        untrack(() => (this.status = undefined));
      });
    });
    return () => {
      stop();
      this.services = undefined;
    };
  }

  /** One ask at a time; one more after it if asked meanwhile. */
  private async refresh(): Promise<void> {
    if (this.asking) {
      this.again = true;
      return;
    }
    const services = this.services;
    if (services === undefined) return;
    this.asking = true;
    const cwd = services.daemon.cwd;
    try {
      const answer = await services.git.status(cwd).catch(() => null);
      if (cwd !== services.daemon.cwd) return;
      this.status = answer === null ? undefined : parseStatus(answer.out, answer.prefix);
    } finally {
      this.asking = false;
      if (this.again) {
        this.again = false;
        void this.refresh();
      }
    }
  }
}

export const git = new Git();

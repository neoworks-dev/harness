// The git status of the shown session's directory, for the files panel: asked for when the
// session changes, when its events say files were edited or a turn ended (a moment later, so a
// burst of edits is one ask), and every few seconds anyway because the user edits too.
import { untrack } from "svelte";
import * as host from "../kernel/host/git.ts";
import { parseStatus, type GitStatus } from "../lib/files.ts";
import { suite } from "../state/suite.svelte.ts";
import { activity } from "./activity.svelte.ts";

const DEBOUNCE_MS = 200;
const REFRESH_MS = 10_000;

class Git {
  /** Absent outside a repository, and before the first answer. */
  status = $state.raw<GitStatus>();
  private asking = false;
  private again = false;

  constructor() {
    $effect.root(() => {
      // The directory or the events changed: ask soon.
      $effect(() => {
        const cwd = suite.cwd;
        void activity.changes;
        if (cwd === "") return;
        const timer = setTimeout(() => void this.refresh(), DEBOUNCE_MS);
        return () => clearTimeout(timer);
      });
      $effect(() => {
        if (suite.cwd === "") return;
        const timer = setInterval(() => void this.refresh(), REFRESH_MS);
        return () => clearInterval(timer);
      });
      // Another directory: the old answer is not about it.
      $effect(() => {
        void suite.cwd;
        untrack(() => (this.status = undefined));
      });
    });
  }

  /** One ask at a time; one more after it if asked meanwhile. */
  async refresh(): Promise<void> {
    if (this.asking) {
      this.again = true;
      return;
    }
    this.asking = true;
    const cwd = suite.cwd;
    try {
      const answer = await host.status(cwd).catch(() => null);
      if (cwd !== suite.cwd) return;
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

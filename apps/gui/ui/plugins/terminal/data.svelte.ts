// The session's commands as the terminal panel and the list under the input show them, kept while
// the plugin is mounted (the list shows whether or not the panel is open): the commands from the
// chat's items, and the files of those in the background read every second while they may write.
import { invoke } from "@neoworks/svelte-gpui";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { timeline } from "../transcript/timeline.svelte.ts";
import { applyChunk, isActive, newJob, type JobState } from "./jobs.ts";
import { terminalRuns, withJobs, type TerminalRun } from "./runs.ts";

const POLL_MS = 1000;

class TerminalData {
  /** By the run's id. */
  jobs = $state.raw<Readonly<Record<string, JobState>>>({});
  private base = $state.raw<readonly TerminalRun[]>([]);
  /** The commands, oldest first, a background one's output from its file. */
  runs = $derived(withJobs(this.base, this.jobs));
  private session: string | undefined;
  private reading = false;

  /** Follows the session's items until the returned call stops it. */
  follow(daemon: DaemonService): () => void {
    const stop = $effect.root(() => {
      $effect(() => {
        const base = terminalRuns({
          items: daemon.items,
          timeline: timeline.value,
          cwd: daemon.cwd,
          running: daemon.status.running,
        });
        const session = daemon.sessionId;
        untrack(() => this.sync(base, session));
      });
    });
    const timer = setInterval(() => void this.poll(), POLL_MS);
    return () => {
      stop();
      clearInterval(timer);
    };
  }

  /** Stops a background command; why it couldn't, if it couldn't. */
  async stop(id: string): Promise<string | undefined> {
    const job = this.jobs[id];
    if (job === undefined || !isActive(job)) return "it has ended";
    try {
      await invoke("job_stop", { path: job.path });
      return undefined;
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  }

  private sync(base: readonly TerminalRun[], session: string | undefined): void {
    let jobs = this.jobs;
    if (session !== this.session) {
      this.session = session;
      jobs = {};
    }
    const now = Date.now();
    for (const run of base) {
      if (run.job !== undefined && jobs[run.id]?.path !== run.job) jobs = { ...jobs, [run.id]: newJob(run.job, now) };
    }
    this.base = base;
    if (jobs !== this.jobs) this.jobs = jobs;
  }

  /** Reads on in each file that may still be written; one round at a time. */
  private async poll(): Promise<void> {
    if (this.reading) return;
    this.reading = true;
    try {
      for (const [id, job] of Object.entries(this.jobs)) {
        if (!isActive(job)) continue;
        const chunk = await invoke("job_read", { path: job.path, offset: job.offset }).catch(() => undefined);
        if (chunk === undefined) continue;
        // The session (and its jobs) may have changed while it was read.
        const current = this.jobs[id];
        if (current?.path !== job.path || current.offset !== job.offset) continue;
        this.jobs = { ...this.jobs, [id]: applyChunk(current, chunk, Date.now()) };
      }
    } finally {
      this.reading = false;
    }
  }
}

export const terminalData = new TerminalData();

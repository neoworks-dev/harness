// The GitHub view's state: the repository `gh` finds for the session's directory, the lists of
// issues and pull requests (per filter) and the details opened, each loaded once and kept until
// the directory changes; `reload` asks again. Everything is a read through the host's
// `gh_*` commands. Nothing here can write to GitHub.
import type { Context } from "@neoworks/extension-system";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import type { GithubService } from "../../kernel/services/github.ts";
import { parseUnified } from "../../lib/unified.ts";
import { explain, type Detail, type Kind, type Summary } from "./github.ts";

/** The services the view reads and calls, from the plugin that started it. */
interface Services {
  daemon: DaemonService;
  host: GithubService;
  /** What shows a diff beside the chat: the `viewer` service. */
  viewer: Context["viewer"];
}

/** What a load is in: working, done or failed with something to tell the user. */
export type Loaded<T> =
  { status: "loading" } | { status: "ready"; value: T } | { status: "failed"; message: string };

function failure(error: unknown): { status: "failed"; message: string } {
  const code = (error as { code?: string } | undefined)?.code;
  const message = error instanceof Error ? error.message : String(error);
  return { status: "failed", message: explain(code, message) };
}

const listKey = (kind: Kind, all: boolean, query: string): string => `${kind} ${all} ${query}`;
const itemKey = (kind: Kind, number: number): string => `${kind} ${number}`;

class GitHub {
  /** Ctrl-R: the view is shown in place of the chat. */
  shown = $state(false);
  tab = $state<Kind>("issue");
  /** Closed ones too. */
  all = $state(false);
  /** GitHub search syntax; applied when the user presses Enter in the field. */
  query = $state("");
  /** The item shown in the detail, or none for the list. */
  open = $state.raw<{ kind: Kind; number: number }>();
  /** The row the keys are on in the shown list. */
  selected = $state(0);

  private repoState = $state.raw<Loaded<string>>();
  private lists = $state.raw<Readonly<Record<string, Loaded<Summary[]>>>>({});
  private details = $state.raw<Readonly<Record<string, Loaded<Detail>>>>({});
  private diffs: Record<string, string> = {};
  /** The directory what is cached belongs to. */
  private cwd = "";
  private services?: Services;

  /** Follows the session's directory and what the view is asked for, until the returned call stops it. */
  start(services: Services): () => void {
    const { daemon } = services;
    this.services = services;
    const stop = $effect.root(() => {
      // Another directory is another repository.
      $effect(() => {
        const cwd = daemon.cwd;
        untrack(() => this.reset(cwd));
      });
      // Opening the view starts what it needs.
      $effect(() => {
        if (!this.shown || daemon.sessionId === undefined) return;
        void this.tab;
        void this.all;
        void this.query;
        untrack(() => this.ensureList());
      });
    });
    return () => {
      stop();
      this.services = undefined;
    };
  }

  private get need(): Services {
    if (this.services === undefined) throw new Error("the GitHub view is not started");
    return this.services;
  }

  private reset(cwd: string): void {
    this.cwd = cwd;
    this.repoState = undefined;
    this.lists = {};
    this.details = {};
    this.diffs = {};
    this.open = undefined;
    this.selected = 0;
  }

  get repo(): Loaded<string> | undefined {
    return this.repoState;
  }

  /** The shown tab's list, if it was asked for. */
  get list(): Loaded<Summary[]> | undefined {
    return this.lists[listKey(this.tab, this.all, this.query)];
  }

  detail(kind: Kind, number: number): Loaded<Detail> | undefined {
    return this.details[itemKey(kind, number)];
  }

  /** The repository first, then the list. A failure stays until `reload`. */
  ensureList(): void {
    const repo = this.repoState;
    if (repo === undefined) {
      void this.loadRepo();
    } else if (repo.status === "ready" && this.list === undefined) {
      void this.loadList(repo.value);
    }
  }

  reload(): void {
    if (this.repoState?.status === "failed") this.repoState = undefined;
    const repo = this.repoState;
    if (repo?.status === "ready") {
      if (this.open) void this.loadDetail(repo.value, this.open.kind, this.open.number);
      else void this.loadList(repo.value);
    } else {
      this.ensureList();
    }
  }

  private async loadRepo(): Promise<void> {
    const cwd = this.cwd;
    this.repoState = { status: "loading" };
    try {
      const name = await this.need.host.repo(cwd);
      if (cwd !== this.cwd) return;
      this.repoState = { status: "ready", value: name };
      this.ensureList();
    } catch (error) {
      if (cwd === this.cwd) this.repoState = failure(error);
    }
  }

  private async loadList(repo: string): Promise<void> {
    const { cwd, tab, all, query } = {
      cwd: this.cwd,
      tab: this.tab,
      all: this.all,
      query: this.query,
    };
    const key = listKey(tab, all, query);
    const previous = this.lists[key];
    // A reload keeps the rows on screen until the new ones arrive.
    if (previous?.status !== "ready") this.lists = { ...this.lists, [key]: { status: "loading" } };
    try {
      const args = { cwd, repo, all, query };
      const rows = await (tab === "issue" ? this.need.host.issues(args) : this.need.host.prs(args));
      if (cwd !== this.cwd) return;
      this.lists = { ...this.lists, [key]: { status: "ready", value: rows } };
      this.selected = Math.min(this.selected, Math.max(0, rows.length - 1));
    } catch (error) {
      if (cwd === this.cwd) this.lists = { ...this.lists, [key]: failure(error) };
    }
  }

  /** Shows an item, loading it when it is missing or older than the list says. */
  openItem(kind: Kind, summary: Summary): void {
    const repo = this.repoState;
    if (repo?.status !== "ready") return;
    this.open = { kind, number: summary.number };
    const known = this.detail(kind, summary.number);
    const stale = known?.status !== "ready" || known.value.updatedMs < summary.updatedMs;
    if (stale) void this.loadDetail(repo.value, kind, summary.number);
  }

  back(): void {
    this.open = undefined;
  }

  private async loadDetail(repo: string, kind: Kind, number: number): Promise<void> {
    const cwd = this.cwd;
    const key = itemKey(kind, number);
    if (this.details[key]?.status !== "ready") {
      this.details = { ...this.details, [key]: { status: "loading" } };
    }
    try {
      const args = { cwd, repo, number };
      const detail = await (kind === "issue"
        ? this.need.host.issue(args)
        : this.need.host.pr(args));
      if (cwd !== this.cwd) return;
      this.details = { ...this.details, [key]: { status: "ready", value: detail } };
    } catch (error) {
      if (cwd === this.cwd) this.details = { ...this.details, [key]: failure(error) };
    }
  }

  /**
   * Hands an item to the model: a line of instruction and the item with its conversation, sent
   * as the user's message. The view closes so the answer is in sight.
   */
  async send(kind: Kind, number: number): Promise<void> {
    const repo = this.repoState;
    if (repo?.status !== "ready") return;
    try {
      const args = { cwd: this.cwd, repo: repo.value, number };
      const text = await (kind === "issue"
        ? this.need.host.issueText(args)
        : this.need.host.prText(args));
      this.shown = false;
      await this.need.daemon.send(text);
    } catch (error) {
      this.need.daemon.error = failure(error).message;
    }
  }

  /** A pull request's diff beside the chat, a section for each file. */
  async showDiff(number: number): Promise<void> {
    const repo = this.repoState;
    if (repo?.status !== "ready") return;
    try {
      const key = String(number);
      this.diffs[key] ??= await this.need.host.prDiff({ cwd: this.cwd, repo: repo.value, number });
      this.need.viewer.showPatch(`#${number} ${repo.value}`, parseUnified(this.diffs[key] ?? ""));
    } catch (error) {
      this.need.daemon.error = failure(error).message;
    }
  }
}

export const github = new GitHub();

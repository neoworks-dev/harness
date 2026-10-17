// What the viewer holds: whether it is open and how wide, its tabs (the diff and the files opened
// from it), what the diff tab lists and how, and the lines selected for a comment. The components
// draw it and the plugin's keys and service call it, so they agree on what is open.
import type { Context } from "@neoworks/extension-system";
import { FileError, files as disk } from "@neoworks/svelte-gpui";
import { untrack } from "svelte";
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import type { GitService } from "../../kernel/services/git.ts";
import type { DiffComponent } from "@suite/protocol/types";
import { diffStats, gutterWidth } from "../../lib/diff.ts";
import { parseStatus, relFromPrefix, relativize } from "../../lib/git-status.ts";
import { parseUnified, type FileDiff } from "../../lib/unified.ts";
import { splitLines } from "./linediff.ts";
import { quoteDiff, quoteFile, select, span, type Quote, type Selection } from "./selection.ts";
import { NO_TURN, applyTurn, turnDiffs, turnEdits, type Turn } from "./turn.ts";

export const MIN_WIDTH = 360;
export const DEFAULT_WIDTH = 540;

/** Files bigger than this are not read. */
const FILE_BYTES = 5 * 1024 * 1024;
/** A file shows this many lines at most. */
export const FILE_LINES = 20_000;
/** How long after the first sign that git changed the diff is asked for, so a burst is one ask. */
const DEBOUNCE_MS = 200;

/** What the diff tab lists: git's view of the directory, or a patch handed over (a pull request). */
export type Source =
  { kind: "changes" } | { kind: "patch"; title: string; files: readonly FileDiff[] };
/** The files git sees changed (uncommitted), or the ones the agent edited in the latest turn. */
export type Scope = "uncommitted" | "turn";
export type Layout = "unified" | "split";

export type Tab = { id: string; kind: "diff" } | { id: string; kind: "file"; path: string };

const DIFF_TAB: Tab = { id: "diff", kind: "diff" };
const NO_FILES: readonly FileDiff[] = [];

/** Git's answer for the working tree: every changed file with its diff, or why there is none. */
interface Changes {
  files: readonly FileDiff[];
  error?: string;
}

/** A file tab's text, as far as it is read. */
export type FileText =
  | { status: "loading" }
  | { status: "ready"; lines: readonly string[]; total: number }
  | { status: "error"; message: string };

interface Services {
  daemon: DaemonService;
  git: GitService;
  comments: Context["comments"];
}

/** The name a selection in the diff of `path` goes by. */
export const diffView = (path: string): string => `diff:${path}`;
/** The name a selection in the file tab of `path` goes by. */
export const fileView = (path: string): string => `file:${path}`;

function failure(error: unknown): string {
  if (error instanceof FileError && error.code === "NotFound") return "The file does not exist";
  if (error instanceof FileError && error.code === "InvalidData") return "Not a text file";
  if (error instanceof Error) return error.message;
  return String(error);
}

class Viewer {
  /** The panel is open. */
  shown = $state(false);
  /** The panel has the conversation's place. */
  maximized = $state(false);
  width = $state(DEFAULT_WIDTH);
  tabs = $state.raw<readonly Tab[]>([DIFF_TAB]);
  active = $state("diff");
  source = $state.raw<Source>({ kind: "changes" });
  scope = $state<Scope>("uncommitted");
  layout = $state<Layout>("unified");
  wrap = $state(false);
  /** Paths whose lines the diff tab hides. */
  collapsed = $state.raw<ReadonlySet<string>>(new Set());
  /** The unmodified runs the user opened, as `path` and the run's id. */
  unfolded = $state.raw<ReadonlySet<string>>(new Set());
  /** The last reading of the working tree. */
  changes = $state.raw<Changes>();
  /** Git is being asked. */
  busy = $state(false);
  selection = $state.raw<Selection>();
  /** The comment being written under the selection. */
  draft = $state("");
  /** The file the diff tab should bring into view: a new `seq` asks again. */
  reveal = $state.raw<{ path: string; seq: number }>();
  texts = $state.raw<Readonly<Record<string, FileText>>>({});

  private readonly turnFold = new Folded<Turn>(NO_TURN, (state, record, daemon) =>
    applyTurn(state, record, daemon.cwd),
  );
  private services?: Services;
  private asking = false;
  private again = false;
  private revealSeq = 0;

  /** What the agent did this turn. */
  get turn(): Turn {
    return this.turnFold.value;
  }

  /** The files the diff tab lists, by its source and scope. */
  diffs = $derived.by((): readonly FileDiff[] => {
    if (this.source.kind === "patch") return this.source.files;
    if (this.scope === "turn") return turnDiffs(turnEdits(this.turn));
    return this.changes?.files ?? NO_FILES;
  });

  /** Lines added and removed in all of them. */
  totals = $derived.by(() => {
    let added = 0;
    let removed = 0;
    for (const file of this.diffs) {
      const stats = diffStats(file.lines);
      added += stats.added;
      removed += stats.removed;
    }
    return { added, removed };
  });

  /** The most digits a line number of the diffs has. */
  digits = $derived(Math.max(1, ...this.diffs.map((file) => gutterWidth(file.lines))));

  /** Why git could not tell, while that is what the diff tab would list. */
  problem = $derived(
    this.source.kind === "changes" && this.scope === "uncommitted"
      ? this.changes?.error
      : undefined,
  );

  /** The tab shown. */
  get tab(): Tab {
    return this.tabs.find((tab) => tab.id === this.active) ?? DIFF_TAB;
  }

  /** The selection is in what the panel shows, so its comment editor is. */
  get editing(): boolean {
    const { selection, tab } = this;
    if (!this.shown || selection === undefined) return false;
    if (tab.kind === "diff") return selection.view.startsWith("diff:");
    return selection.view === fileView(tab.path);
  }

  /** Serves the services, and keeps the diff current while the panel is open, until the call stops it. */
  start(services: Services): () => void {
    this.services = services;
    const { daemon } = services;
    const stop = $effect.root(() => {
      // Something changed that git may say differently about: ask soon.
      $effect(() => {
        if (!this.shown) return;
        void daemon.cwd;
        void this.scope;
        void this.source;
        void this.turn.version;
        const timer = setTimeout(() => void this.refresh(), DEBOUNCE_MS);
        return () => clearTimeout(timer);
      });
      // Another session: nothing of the last one.
      $effect(() => {
        void daemon.sessionId;
        untrack(() => this.reset());
      });
    });
    return () => {
      stop();
      this.services = undefined;
    };
  }

  /** Follows the session's events, for the latest turn. */
  follow(daemon: DaemonService): void {
    this.turnFold.follow(daemon);
  }

  private reset(): void {
    this.changes = undefined;
    this.source = { kind: "changes" };
    this.tabs = [DIFF_TAB];
    this.active = "diff";
    this.collapsed = new Set();
    this.unfolded = new Set();
    this.texts = {};
    this.clearSelection();
  }

  // ---- showing ----

  toggle(): void {
    this.shown = !this.shown;
  }

  close(): void {
    this.shown = false;
    this.maximized = false;
  }

  /** Opens the diff of the directory's uncommitted changes, with `path` brought into view. */
  showChanges(path?: string): void {
    this.source = { kind: "changes" };
    this.scope = "uncommitted";
    this.open("diff");
    if (path !== undefined) this.reveal = { path, seq: (this.revealSeq += 1) };
  }

  /** Opens a patch (a pull request's diff) in the diff tab. */
  showPatch(title: string, patch: readonly FileDiff[]): void {
    this.source = { kind: "patch", title, files: patch };
    this.collapsed = new Set();
    this.unfolded = new Set();
    this.open("diff");
  }

  /** Opens a diff the agent put in the chat (one file) in the diff tab. */
  showDiff(diff: DiffComponent): void {
    const path = relativize(this.services?.daemon.cwd ?? "", diff.path ?? "diff");
    this.showPatch(path, [{ path, lines: diff.lines }]);
  }

  /** Opens a file of the session's directory in a tab. */
  openFile(path: string): void {
    if (!this.tabs.some((tab) => tab.id === fileView(path))) {
      this.tabs = [...this.tabs, { id: fileView(path), kind: "file", path }];
    }
    this.open(fileView(path));
    void this.readFile(path);
  }

  private open(tab: string): void {
    this.shown = true;
    this.activate(tab);
  }

  activate(id: string): void {
    if (this.active !== id) this.clearSelection();
    this.active = id;
  }

  closeTab(id: string): void {
    const at = this.tabs.findIndex((tab) => tab.id === id);
    const closing = this.tabs[at];
    if (closing === undefined || closing.kind === "diff") return;
    this.tabs = this.tabs.filter((tab) => tab.id !== id);
    if (this.active === id) this.activate(this.tabs[Math.max(0, at - 1)]?.id ?? "diff");
    this.texts = Object.fromEntries(
      Object.entries(this.texts).filter(([path]) => path !== closing.path),
    );
    if (this.selection?.view === fileView(closing.path)) this.clearSelection();
  }

  // ---- the diff tab ----

  setScope(scope: Scope): void {
    this.scope = scope;
  }

  toggleFile(path: string): void {
    const next = new Set(this.collapsed);
    if (!next.delete(path)) next.add(path);
    this.collapsed = next;
  }

  /** Every file folded to its header, or, when all are, every one open. */
  toggleAll(): void {
    const paths = this.diffs.map((file) => file.path);
    this.collapsed = this.allCollapsed ? new Set() : new Set(paths);
  }

  get allCollapsed(): boolean {
    return this.diffs.length > 0 && this.diffs.every((file) => this.collapsed.has(file.path));
  }

  /** Opens the run of unmodified lines `id` of `path`. */
  unfold(path: string, id: number): void {
    this.unfolded = new Set([...this.unfolded, `${path}\0${id}`]);
  }

  isUnfolded(path: string, id: number): boolean {
    return this.unfolded.has(`${path}\0${id}`);
  }

  // ---- reading ----

  /** Asks git again, and re-reads the open files. */
  async refresh(): Promise<void> {
    if (this.source.kind === "changes" && this.scope === "uncommitted") await this.readChanges();
    for (const tab of this.tabs) if (tab.kind === "file") void this.readFile(tab.path);
  }

  /** One ask at a time; one more after it if asked meanwhile. */
  private async readChanges(): Promise<void> {
    if (this.asking) {
      this.again = true;
      return;
    }
    const services = this.services;
    if (services === undefined) return;
    this.asking = true;
    this.busy = true;
    const cwd = services.daemon.cwd;
    try {
      const read = await this.readWorkingTree(services.git, cwd);
      if (cwd === services.daemon.cwd) this.changes = read;
    } finally {
      this.asking = false;
      this.busy = false;
      if (this.again) {
        this.again = false;
        void this.readChanges();
      }
    }
  }

  private async readWorkingTree(git: GitService, cwd: string): Promise<Changes> {
    try {
      const answer = await git.status(cwd, true);
      if (answer === null)
        return { files: NO_FILES, error: "This directory is not in a git repository." };
      const status = parseStatus(answer.out, answer.prefix);
      const entries = status.entries
        .map((entry) => ({ rel: relFromPrefix(status.prefix, entry.path), change: entry.change }))
        .filter((entry) => !entry.rel.endsWith("/"));
      const listed: FileDiff[] = [];
      for (const entry of entries) {
        const text = await git.diff({
          cwd,
          path: entry.rel,
          untracked: entry.change === "untracked",
          full: true,
        });
        // The whole file is one hunk; its header says nothing the lines do not.
        const lines = parseUnified(text)
          .flatMap((file) => file.lines)
          .filter((line) => line.kind !== "hunk");
        listed.push({ path: entry.rel, lines });
      }
      return { files: listed };
    } catch (error) {
      return { files: NO_FILES, error: failure(error) };
    }
  }

  /** `path` as the host can read it: from the session's directory unless it is absolute. */
  private absolute(path: string): string {
    if (path.startsWith("/") || path.startsWith("~")) return path;
    return `${this.services?.daemon.cwd ?? ""}/${path}`;
  }

  private setText(path: string, text: FileText): void {
    this.texts = { ...this.texts, [path]: text };
  }

  /** Reads a file for its tab: capped in size and in lines, with a message when it cannot be shown. */
  async readFile(path: string): Promise<void> {
    if (this.texts[path] === undefined) this.setText(path, { status: "loading" });
    const target = this.absolute(path);
    try {
      const info = await disk.stat(target);
      if (info.kind !== "file") throw new Error("Not a file");
      if (info.size > FILE_BYTES) {
        const megabytes = (info.size / (1024 * 1024)).toFixed(1);
        throw new Error(`The file is ${megabytes} MB, too large to show`);
      }
      const all = splitLines(await disk.readText(target));
      const lines = all.length > FILE_LINES ? all.slice(0, FILE_LINES) : all;
      if (this.tabs.some((tab) => tab.id === fileView(path))) {
        this.setText(path, { status: "ready", lines, total: all.length });
      }
    } catch (error) {
      if (this.tabs.some((tab) => tab.id === fileView(path))) {
        this.setText(path, { status: "error", message: failure(error) });
      }
    }
  }

  // ---- comments ----

  /** Pressing a line number: selects the line, or with shift stretches the selection to it. */
  press(view: string, index: number, extend: boolean): void {
    this.selection = select(this.selection, view, index, extend);
  }

  clearSelection(): void {
    this.selection = undefined;
    this.draft = "";
  }

  /** Where the selected lines are, as a comment names them; undefined when they are gone. */
  get place(): Quote | undefined {
    const { selection } = this;
    if (selection === undefined) return undefined;
    const { from, to } = span(selection);
    const path = selection.view.slice(selection.view.indexOf(":") + 1);
    if (selection.view.startsWith("diff:")) {
      const file = this.diffs.find((candidate) => candidate.path === path);
      return file && quoteDiff(path, file.lines, from, to);
    }
    const text = this.texts[path];
    return text?.status === "ready" ? quoteFile(path, text.lines, from, to) : undefined;
  }

  /** Hands the comment to the input, which sends it with the next message. */
  submit(): void {
    const text = this.draft.trim();
    const quote = this.place;
    if (text === "" || quote === undefined) return;
    this.services?.comments.add({ ...quote, text });
    this.clearSelection();
  }
}

export const viewer = new Viewer();

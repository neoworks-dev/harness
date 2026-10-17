// The workspace card's state: whether it is shown, the editor last used (both kept while the app
// runs), what the host says of the session's directory (asked for again every few seconds, when
// the agent edits files and after our own commits) and the actions the card's buttons run. The
// host does the work; a failure becomes a line under the card.
import type { Context } from "@neoworks/extension-system";
import { clipboard } from "@neoworks/svelte-gpui";
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import * as host from "./host.ts";
import {
  ASK_AGENT_TO_COMMIT,
  NO_COUNTS,
  committedMessage,
  errorText,
  preferredEditor,
  primaryAction,
  sumStat,
  type Counts,
  type Editor,
  type Overview,
} from "./workspace.ts";

const REFRESH_MS = 5000;
/** After a burst of edits, one ask. */
const POKE_MS = 400;
const NOTICE_MS = 7000;

interface Services {
  daemon: DaemonService;
  /** What shows the diff of the changes: the `viewer` service. */
  viewer: Context["viewer"];
}

/** A line under the card. */
export interface Notice {
  text: string;
  error: boolean;
}

class Workspace {
  /** Hidden by the toolbar button or the key. */
  hidden = $state(false);
  /** The id of the editor last used. */
  lastEditor = $state<string>();
  editors = $state.raw<readonly Editor[]>([]);
  hostname = $state("");
  /** Absent before the first answer, null outside a repository. */
  overview = $state.raw<Overview | null>();
  /** The local branches, as of the branch menu last opened. */
  branches = $state.raw<readonly string[]>([]);
  notice = $state.raw<Notice>();
  /** What is running, in the words of the line under the card. */
  busy = $state<string>();
  /** The commit message field is open; Enter commits, and pushes if `push`. */
  committing = $state.raw<{ push: boolean }>();
  message = $state("");

  private services?: Services;
  private asking = false;
  private again = false;
  private pokeTimer?: ReturnType<typeof setTimeout>;
  private noticeTimer?: ReturnType<typeof setTimeout>;

  /** The editor the primary button opens. */
  get editor(): Editor | undefined {
    return preferredEditor(this.editors, this.lastEditor);
  }

  /** The working tree's change against HEAD. */
  get counts(): Counts {
    const overview = this.overview;
    return overview ? sumStat(overview.numstat, overview.untracked) : NO_COUNTS;
  }

  private get need(): Services {
    if (this.services === undefined) throw new Error("the workspace card is not started");
    return this.services;
  }

  /** Follows the session's directory until the returned call stops it. */
  start(services: Services): () => void {
    const { daemon } = services;
    this.services = services;
    host
      .hostname()
      .then((name) => (this.hostname = name))
      .catch(() => {});
    host
      .editors()
      .then((found) => (this.editors = found))
      .catch(() => {});
    const stop = $effect.root(() => {
      // Another directory: what was known is about the old one.
      $effect(() => {
        void daemon.cwd;
        untrack(() => this.reset());
      });
      $effect(() => {
        if (daemon.cwd === "" || this.hidden) return;
        const first = setTimeout(() => void this.refresh(), 0);
        const timer = setInterval(() => void this.refresh(), REFRESH_MS);
        return () => {
          clearTimeout(first);
          clearInterval(timer);
        };
      });
    });
    return () => {
      stop();
      clearTimeout(this.pokeTimer);
      clearTimeout(this.noticeTimer);
      this.services = undefined;
    };
  }

  private reset(): void {
    this.overview = undefined;
    this.branches = [];
    this.notice = undefined;
    this.committing = undefined;
    this.message = "";
  }

  /** Something probably changed in the directory (the agent edited, a turn ended): ask soon. */
  poke(): void {
    if (this.services === undefined || this.hidden) return;
    clearTimeout(this.pokeTimer);
    this.pokeTimer = setTimeout(() => void this.refresh(), POKE_MS);
  }

  /** One ask at a time; one more after it if asked meanwhile. */
  async refresh(): Promise<void> {
    if (this.asking) {
      this.again = true;
      return;
    }
    const services = this.services;
    if (services === undefined) return;
    this.asking = true;
    const cwd = services.daemon.cwd;
    try {
      const answer = await host.overview(cwd).catch(() => null);
      if (cwd === services.daemon.cwd) this.overview = answer;
    } finally {
      this.asking = false;
      if (this.again) {
        this.again = false;
        void this.refresh();
      }
    }
  }

  /** Shows a line under the card, and takes it away after a while. */
  tell(text: string, error = false): void {
    clearTimeout(this.noticeTimer);
    this.notice = { text, error };
    this.noticeTimer = setTimeout(() => (this.notice = undefined), NOTICE_MS);
  }

  /** Runs `action` with `what` shown under the card; its failure is told there. */
  private async work(what: string, action: () => Promise<void>): Promise<void> {
    if (this.busy !== undefined) return;
    clearTimeout(this.noticeTimer);
    this.notice = undefined;
    this.busy = what;
    try {
      await action();
    } catch (error) {
      this.tell(errorText(error), true);
    } finally {
      this.busy = undefined;
    }
  }

  /** Opens the session's directory in an editor (the preferred one without an id). */
  async openEditor(id?: string): Promise<void> {
    const editor = id === undefined ? this.editor : this.editors.find((e) => e.id === id);
    if (editor === undefined) return;
    this.lastEditor = editor.id;
    try {
      await host.openInEditor(editor.id, this.need.daemon.cwd);
    } catch (error) {
      this.tell(errorText(error), true);
    }
  }

  async openFolder(): Promise<void> {
    try {
      await host.openFolder(this.need.daemon.cwd);
    } catch (error) {
      this.tell(errorText(error), true);
    }
  }

  async copy(text: string, what: string): Promise<void> {
    try {
      await clipboard.write(text);
      this.tell(`Copied ${what}`);
    } catch (error) {
      this.tell(errorText(error), true);
    }
  }

  /** Asks the host for the local branches; the menu shows them when they arrive. */
  async loadBranches(): Promise<void> {
    const cwd = this.need.daemon.cwd;
    try {
      const answer = await host.branches(cwd);
      if (cwd === this.need.daemon.cwd) this.branches = answer.branches;
    } catch (error) {
      this.tell(errorText(error), true);
    }
  }

  /** Switches branch. The host refuses when the tree has changes, and says so. */
  async switchTo(branch: string): Promise<void> {
    await this.work(`Switching to ${branch}…`, async () => {
      await host.checkout(this.need.daemon.cwd, branch);
      this.tell(`Switched to ${branch}`);
      await this.refresh();
    });
  }

  /** What the primary button of the commit split does now. */
  primary(): void {
    const overview = this.overview;
    if (!overview) return;
    const action = primaryAction(overview);
    if (action.kind === "commit-push") this.beginCommit(true);
    else if (action.kind === "push") void this.push();
  }

  /** Opens the message field. */
  beginCommit(push: boolean): void {
    clearTimeout(this.noticeTimer);
    this.notice = undefined;
    this.message = "";
    this.committing = { push };
  }

  cancelCommit(): void {
    this.committing = undefined;
    this.message = "";
  }

  /** Commits all changes with the message in the field, and pushes if it was opened for that. */
  async submitCommit(): Promise<void> {
    const committing = this.committing;
    if (committing === undefined) return;
    const message = this.message.trim();
    if (message === "") {
      this.tell("Write a commit message", true);
      return;
    }
    await this.work(committing.push ? "Committing and pushing…" : "Committing…", async () => {
      const done = await host.commit(this.need.daemon.cwd, message, committing.push);
      this.cancelCommit();
      this.tell(committedMessage(done), done.failed);
      await this.refresh();
    });
  }

  /** Pushes the commits that were not, when nothing else is left to commit. */
  async push(): Promise<void> {
    await this.work("Pushing…", async () => {
      const done = await host.commit(this.need.daemon.cwd, "", true);
      this.tell(committedMessage(done), done.failed);
      await this.refresh();
    });
  }

  /** Leaves the commit to the agent: it is asked in the conversation. */
  async askAgent(): Promise<void> {
    try {
      await this.need.daemon.send(ASK_AGENT_TO_COMMIT);
      this.tell("Asked the agent to commit");
    } catch (error) {
      this.tell(errorText(error), true);
    }
  }

  /** Everything changed against HEAD, in the viewer. */
  showChanges(): void {
    this.need.viewer.showChanges();
  }
}

export const workspace = new Workspace();

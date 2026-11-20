// The edit under review in the chat's place (PermitPane): the requests that wait, which one is
// shown, the cursor and the visual run over its lines, the comments on them and the one being
// written. Enter accepts, with the comments as the answer's note; declining carries them too.
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import {
  answers,
  applyPermit,
  nextChangeSpot,
  noteOf,
  runOf,
  spotsOf,
  type EditComment,
  type PendingEdit,
  type Spot,
} from "./permit.ts";

const NONE: readonly PendingEdit[] = [];

class Permit {
  private readonly fold = new Folded<readonly PendingEdit[]>(NONE, (state, record, daemon) =>
    applyPermit(state, record, daemon.cwd),
  );
  private daemon?: DaemonService;
  /** Requests put aside with Esc: answered from the chat instead. */
  aside = $state.raw<ReadonlySet<string>>(new Set());
  /** The request the cursor and comments below are for. */
  private of = $state<string>();
  /** The cursor's line, and where a visual run (V, shift) began. */
  head = $state(0);
  anchor = $state(0);
  visual = $state(false);
  comments = $state.raw<readonly EditComment[]>([]);
  /** The comment being written, on the lines it was opened on. */
  draft = $state.raw<{ file: number; from: number; to: number } | undefined>();
  text = $state("");
  /** The line the list should bring into view: a new `seq` asks again. */
  jump = $state.raw<{ at: number; seq: number }>({ at: 0, seq: 0 });
  /** Sending the answer. */
  busy = $state(false);

  /** The request shown: the oldest not put aside. */
  get current(): PendingEdit | undefined {
    return this.fold.value.find((edit) => !this.aside.has(edit.id));
  }

  /** How many requests wait, the shown one included. */
  get waiting(): number {
    return this.fold.value.filter((edit) => !this.aside.has(edit.id)).length;
  }

  get spots(): readonly Spot[] {
    const edit = this.current;
    return edit === undefined ? [] : spotsOf(edit.files);
  }

  /** The lines under the cursor: the visual run, or its line. */
  get run(): { file: number; from: number; to: number } | undefined {
    return runOf(this.spots, this.visual ? this.anchor : this.head, this.head);
  }

  follow(daemon: DaemonService): void {
    this.daemon = daemon;
    this.fold.follow(daemon);
  }

  /** A new request shown: the cursor on its first change, nothing written on it yet. */
  sync(): void {
    const edit = this.current;
    if (edit?.id === this.of) return;
    this.of = edit?.id;
    this.comments = [];
    this.draft = undefined;
    this.text = "";
    this.visual = false;
    this.busy = false;
    const first = edit === undefined ? undefined : nextChangeSpot(edit.files, -1, 1);
    this.moveTo(first ?? 0, false);
  }

  moveTo(at: number, extend: boolean): void {
    const last = Math.max(0, this.spots.length - 1);
    const to = Math.min(last, Math.max(0, at));
    if (!extend && !this.visual) this.anchor = to;
    else if (extend && !this.visual) {
      this.visual = true;
    }
    this.head = to;
    this.jump = { at: to, seq: this.jump.seq + 1 };
  }

  move(delta: number, extend = false): void {
    this.moveTo(this.head + delta, extend);
  }

  /** n / N, ]c / [c: the next or previous run of changed lines. */
  nextChange(direction: 1 | -1): void {
    const edit = this.current;
    const to = edit === undefined ? undefined : nextChangeSpot(edit.files, this.head, direction);
    if (to !== undefined) this.moveTo(to, false);
  }

  /** A press on a line: the cursor there, or with shift the run stretched to it. */
  press(at: number, extend: boolean): void {
    if (!extend) this.visual = false;
    this.moveTo(at, extend);
  }

  toggleVisual(): void {
    this.visual = !this.visual;
    this.anchor = this.head;
  }

  /** c: a comment on the lines under the cursor; its editor opens under them. */
  startComment(): void {
    const run = this.run;
    if (run === undefined) return;
    const said = this.comments.find(
      (comment) => comment.file === run.file && comment.from === run.from && comment.to === run.to,
    );
    this.draft = run;
    this.text = said?.text ?? "";
    this.visual = false;
  }

  /** The comment written is kept (replacing one on the very same lines); empty, it drops that one. */
  saveComment(): void {
    const draft = this.draft;
    if (draft === undefined) return;
    const text = this.text.trim();
    const others = this.comments.filter(
      (comment) => !(comment.file === draft.file && comment.from === draft.from && comment.to === draft.to),
    );
    this.comments = text === "" ? others : [...others, { ...draft, text }];
    this.draft = undefined;
    this.text = "";
  }

  cancelComment(): void {
    this.draft = undefined;
    this.text = "";
  }

  removeComment(at: number): void {
    this.comments = this.comments.filter((_, index) => index !== at);
  }

  /** Esc: back to the chat; the request stays there to answer. */
  putAside(): void {
    const edit = this.current;
    if (edit !== undefined) this.aside = new Set([...this.aside, edit.id]);
  }

  /** The requests put aside come back. */
  bringBack(): boolean {
    if (this.aside.size === 0) return false;
    this.aside = new Set();
    return true;
  }

  /** Enter (accept) or x (decline): the answer, with the comments as its note. */
  async answer(accept: boolean): Promise<void> {
    const edit = this.current;
    const daemon = this.daemon;
    if (edit === undefined || daemon === undefined || this.busy) return;
    const { accept: yes, decline: no } = answers(edit.options);
    const option = accept ? yes : no;
    if (option === undefined) return;
    if (this.draft !== undefined) this.saveComment();
    const note = noteOf(edit.files, this.comments);
    this.busy = true;
    try {
      await daemon.action(edit.ref, "neo.permit", { id: edit.id, option: option.id, note });
    } catch (error) {
      daemon.error = error instanceof Error ? error.message : String(error);
      this.busy = false;
    }
  }
}

export const permit = new Permit();

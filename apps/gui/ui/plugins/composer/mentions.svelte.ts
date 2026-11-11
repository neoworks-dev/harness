// The `@` menu above the input (lib/mention.ts): the files the word at the cursor could be, asked
// of the host for the session's directory as the word changes. The input keeps the keyboard; its
// keys call `move`, `pick` and `close`.
import { invoke } from "@neoworks/svelte-gpui";
import { accept, atCursor, names, type Candidate } from "../../lib/mention.ts";
import { moveSelection } from "../../lib/commands.ts";

const NONE: readonly Candidate[] = [];

class Mentions {
  /** Where the input's cursor is, as of the last change the input reported. */
  cursor = $state(0);
  /** The highlighted row. */
  selected = $state(0);
  /** What the host answered, and for which word. */
  private found = $state.raw<{ key: string; candidates: readonly Candidate[] }>();
  /** The text the menu was closed (Esc) for; typing more opens it again. */
  private closedFor = $state<string>();
  private text = $state("");
  private cwd = $state("");

  /** The word at the cursor, unless the menu was closed for this text. */
  mention = $derived(this.closedFor === this.text ? undefined : atCursor(this.text, this.cursor));
  private key = $derived(this.mention && JSON.stringify([this.cwd, this.mention.query, this.mention.cd]));
  /** The rows the menu shows: what was found for the word at the cursor. */
  candidates = $derived(this.key !== undefined && this.found?.key === this.key ? this.found.candidates : NONE);
  visible = $derived(this.candidates.length > 0);

  /** The input's text, cursor or session directory changed: asks for the word now at the cursor. */
  track(text: string, cursor: number, cwd: string): void {
    const moved = text !== this.text || cursor !== this.cursor;
    this.text = text;
    this.cursor = cursor;
    this.cwd = cwd;
    if (moved) this.selected = 0;
    const mention = this.mention;
    const key = this.key;
    if (mention === undefined || key === undefined || cwd === "" || this.found?.key === key) return;
    invoke("mentions", { cwd, query: mention.query, folders: mention.cd }).then(
      (candidates) => {
        if (this.key === key) this.found = { key, candidates };
      },
      () => {
        if (this.key === key) this.found = { key, candidates: NONE };
      },
    );
  }

  move(delta: number): void {
    this.selected = moveSelection(this.selected, delta, this.candidates.length);
  }

  /** Whether Enter picks a row: not for a `/cd` argument that names a folder already. */
  get entersPick(): boolean {
    return this.mention !== undefined && !names(this.mention);
  }

  /** The text and cursor with the highlighted (or `index`th) row in the word's place. */
  pick(index = this.selected): { text: string; cursor: number } | undefined {
    const mention = this.mention;
    const candidate = this.candidates[Math.min(index, this.candidates.length - 1)];
    if (mention === undefined || candidate === undefined) return undefined;
    return accept(this.text, this.cursor, mention, candidate);
  }

  close(): void {
    this.closedFor = this.text;
  }
}

export const mentions = new Mentions();

// What was sent in a session, walked back with Up and forward with Down, and how it is kept on
// disk (`<data>/suite/gui-history/<session>.jsonl`, one JSON string per line, like the terminal
// app's). The file access itself is in input/history-store.ts; this part needs no window.

/** Entries kept (and loaded) per session. */
export const HISTORY_LIMIT = 500;

/** Walking through earlier inputs with Up and back with Down. */
export class History {
  private entries: string[];
  /** Index being shown; undefined while editing a fresh draft. */
  private position: number | undefined;
  private draft = "";

  constructor(entries: readonly string[] = []) {
    this.entries = entries.slice(-HISTORY_LIMIT);
  }

  all(): readonly string[] {
    return this.entries;
  }

  /** Records a sent input; a repeat of the last one is not added twice. Returns whether it was. */
  push(text: string): boolean {
    this.position = undefined;
    this.draft = "";
    if (text === "" || this.entries.at(-1) === text) return false;
    this.entries.push(text);
    if (this.entries.length > HISTORY_LIMIT) this.entries.shift();
    return true;
  }

  /** Up: the previous entry, remembering `current` as the draft on the way in. */
  older(current: string): string | undefined {
    if (this.entries.length === 0) return undefined;
    let position: number;
    if (this.position === undefined) {
      this.draft = current;
      position = this.entries.length - 1;
    } else if (this.position === 0) {
      return undefined;
    } else {
      position = this.position - 1;
    }
    this.position = position;
    return this.entries[position];
  }

  /** Down: the next entry, or the draft after the newest. */
  newer(): string | undefined {
    if (this.position === undefined) return undefined;
    if (this.position + 1 < this.entries.length) {
      this.position += 1;
      return this.entries[this.position];
    }
    this.position = undefined;
    const draft = this.draft;
    this.draft = "";
    return draft;
  }

  /** Typing ends the walk; the text shown is the new draft. */
  reset(): void {
    this.position = undefined;
  }

  get browsing(): boolean {
    return this.position !== undefined;
  }
}

/** The entries in a history file's text; lines that are not a JSON string are skipped. */
export function parseHistory(text: string): string[] {
  const entries: string[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    try {
      const entry: unknown = JSON.parse(line);
      if (typeof entry === "string") entries.push(entry);
    } catch {
      // A torn last line from an interrupted write: history is a convenience.
    }
  }
  return entries.slice(-HISTORY_LIMIT);
}

/** One entry as the line appended to the file. */
export function historyLine(entry: string): string {
  return `${JSON.stringify(entry)}\n`;
}

/** The file of a session's history, or undefined when the id is not a plain name. */
export function historyFile(directory: string, session: string): string | undefined {
  if (!/^[A-Za-z0-9_-]+$/.test(session)) return undefined;
  return `${directory}/${session}.jsonl`;
}

/**
 * Whether Up or Down should walk the history rather than move the cursor. An `Input` does not
 * say where its cursor is, so the first and last line are guessed: the text is one line that does
 * not wrap (`columns` is how many characters fit a row). Alt-Up/Down walk it regardless.
 */
export function walksHistory(text: string, columns: number, forced: boolean): boolean {
  if (forced) return true;
  return !text.includes("\n") && text.length < columns;
}

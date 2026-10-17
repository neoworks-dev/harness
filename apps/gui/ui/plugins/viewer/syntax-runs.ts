// What a scanner of the viewer's syntax highlighting returns: a line as runs of text with a kind.

export type Kind =
  | "keyword"
  | "string"
  | "number"
  | "comment"
  | "function"
  | "type"
  | "constant"
  | "tag"
  | "attr"
  | "property";

/** A run of a line: `kind` is left out for text that is not coloured. */
export interface Token {
  text: string;
  kind?: Kind;
}

export interface Scanned {
  tokens: Token[];
  /** What the next line starts in; "" is plain code. */
  state: string;
}

/** Reads one line of a language that starts in `state`. */
export type Scanner = (line: string, state: string) => Scanned;

/** Collects the runs of a line, joining neighbours of one kind. */
export class Runs {
  readonly tokens: Token[] = [];

  add(text: string, kind?: Kind): void {
    if (text === "") return;
    const last = this.tokens[this.tokens.length - 1];
    if (last !== undefined && last.kind === kind) last.text += text;
    else this.tokens.push(kind === undefined ? { text } : { text, kind });
  }

  addAll(tokens: readonly Token[]): void {
    for (const token of tokens) this.add(token.text, token.kind);
  }
}

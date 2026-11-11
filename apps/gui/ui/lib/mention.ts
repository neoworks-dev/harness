// `@` file mentions in the input: the word at the cursor starting with `@` opens a list of files
// to put there, found by the host's `mentions` command (crates/mention: `@` lists the session's
// directory, `@src/` a folder in it, anything else is matched loosely). Picking a folder keeps the
// list open in it; picking a file ends the word. `/cd `'s argument completes the same way, with
// folders only and no `@`. Plain functions, tested without a window.

/** The `@word` the cursor is in, or `/cd`'s argument. */
export interface Mention {
  /** Where the word starts: its `@`, or the argument's first character. */
  start: number;
  /** What follows the `@` up to the cursor. */
  query: string;
  /** `/cd`'s argument: folders only, and no `@` in front. */
  cd: boolean;
}

/** One thing to put after the `@`: its path as typed, a folder's ending in `/`. */
export interface Candidate {
  path: string;
  dir: boolean;
}

/** The `@word` ending at `cursor`, when the `@` starts a word; or the argument of a `/cd ` the text starts with. */
export function atCursor(text: string, cursor: number): Mention | undefined {
  const before = text.slice(0, cursor);
  if (before.startsWith("/cd ")) return { start: 4, query: before.slice(4), cd: true };
  const start = Math.max(before.lastIndexOf(" "), before.lastIndexOf("\n"), before.lastIndexOf("\t")) + 1;
  const word = before.slice(start);
  if (!word.startsWith("@")) return undefined;
  const query = word.slice(1);
  // An e-mail address or a second `@` isn't a mention.
  if (query.includes("@")) return undefined;
  return { start, query, cd: false };
}

/** Whether a `/cd` argument names a folder already (`src/`, `..`, nothing): Enter runs the command rather than picking. */
export function names(mention: Mention): boolean {
  if (!mention.cd) return false;
  const { query } = mention;
  return query === "" || query === ".." || query === "~" || query.endsWith("/") || query.endsWith("/..");
}

/** The text with `pick` in the word's place, and where the cursor goes: a folder stays open to go on in, a file ends the word with a space. */
export function accept(
  text: string,
  cursor: number,
  mention: Mention,
  pick: Candidate,
): { text: string; cursor: number } {
  const word = `${mention.cd ? "" : "@"}${pick.path}${pick.dir ? "" : " "}`;
  const head = text.slice(0, mention.start) + word;
  return { text: head + text.slice(cursor), cursor: head.length };
}

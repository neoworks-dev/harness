// Comments on lines of code, written in the viewer and sent with the user's next message. Plain
// data and the text the agent gets, so tests reach them.

/** What a comment is about: lines of a file, quoted as they were when it was written. */
export interface CodeComment {
  id: string;
  /** As the viewer shows it, relative to the session's directory when inside it. */
  path: string;
  /** Which text the line numbers count in: the file as it is, or as it was before the change. */
  side: "new" | "old";
  start: number;
  end: number;
  /** The lines, one string each. A change keeps its mark in front: `+`, `-` or a space. */
  code: readonly string[];
  /** `code` is a diff (it has marks), not plain lines. */
  diff: boolean;
  text: string;
}

/** `src/a.ts:L12-L14`, or `src/a.ts:L12` for one line. */
export function rangeLabel(comment: Pick<CodeComment, "path" | "start" | "end">): string {
  const { path, start, end } = comment;
  return start === end ? `${path}:L${start}` : `${path}:L${start}-L${end}`;
}

/** The first `max` characters of the comment's first line, with an ellipsis when cut. */
export function preview(text: string, max: number): string {
  const line = (text.trim().split("\n")[0] ?? "").trimEnd();
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/** Whether the comment covers line `no` of `side` in `path`. */
export function covers(
  comment: CodeComment,
  path: string,
  side: "new" | "old",
  no: number,
): boolean {
  return comment.path === path && comment.side === side && no >= comment.start && no <= comment.end;
}

/** The language a fence names, from the file's extension. */
function language(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "";
  return name.slice(dot + 1).toLowerCase();
}

/** A fence longer than any run of backticks in the code, so the code cannot close it. */
function fenceFor(lines: readonly string[]): string {
  let longest = 0;
  for (const line of lines) {
    for (const run of line.match(/`+/g) ?? []) longest = Math.max(longest, run.length);
  }
  return "`".repeat(Math.max(3, longest + 1));
}

/** One comment as the agent reads it: where, the quoted lines, then what the user said. */
export function formatComment(comment: CodeComment): string {
  const { path, start, end, side, diff } = comment;
  const lines = start === end ? `line ${start}` : `lines ${start}-${end}`;
  let where = `\`${path}\` ${lines}`;
  if (side === "old") where += " of the old version (before the change)";
  if (diff) where += ' (a diff: "+" added, "-" removed)';
  const fence = fenceFor(comment.code);
  const info = diff ? "diff" : language(path);
  return [`${where}:`, `${fence}${info}`, ...comment.code, fence, comment.text.trim()].join("\n");
}

/** The message to send: what the user typed, then every comment as a block. */
export function formatMessage(text: string, comments: readonly CodeComment[]): string {
  if (comments.length === 0) return text;
  const blocks = comments.map(formatComment).join("\n\n");
  const heading = "Comments on the code:";
  return [text.trim(), `${heading}\n\n${blocks}`].filter((part) => part !== "").join("\n\n");
}

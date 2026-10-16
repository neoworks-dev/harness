// The slash commands the `/` menu lists, and how what is typed picks among them. Plain functions,
// so they are tested without a window. The daemon answers most commands itself (`/model` opens a
// choice card); the `local` ones are handled by the window.

export interface Command {
  name: string;
  /** Argument hint shown after the name. */
  args: string;
  desc: string;
  /** Handled by this window instead of being sent to the daemon. */
  local: boolean;
  /** Pointless without arguments: Enter completes it instead of running it. */
  needsArgs: boolean;
}

function daemon(name: string, args: string, desc: string, needsArgs = false): Command {
  return { name, args, desc, local: false, needsArgs };
}

function local(name: string, desc: string): Command {
  return { name, args: "", desc, local: true, needsArgs: false };
}

export const COMMANDS: readonly Command[] = [
  daemon("clear", "", "start a new conversation in this directory"),
  daemon("model", "[harness] [model]", "show or set the model"),
  daemon("effort", "[harness] [level]", "show or set the reasoning effort"),
  daemon("mode", "[harness] [mode]", "show or set the agent mode"),
  daemon("resume", "[claude|codex]", "continue an earlier Claude Code / Codex conversation"),
  daemon("claude", "<task>", "give a task to Claude Code", true),
  daemon("codex", "<task>", "give a task to Codex", true),
  daemon("harness", "[pi|claude|codex]", "switch this session's agent: pi, claude, codex"),
  daemon(
    "approvals",
    "[ask|review|edits|auto]",
    "ask, review each edit, accept edits, or auto (no value: next)",
  ),
  local("sidebar", "show or hide the sessions sidebar (ctrl-s)"),
  local("expand", "toggle full tool results (ctrl-o)"),
  local("thoughts", "show or hide the agent's thoughts (ctrl-t)"),
  local("github", "GitHub issues and pull requests (ctrl-r)"),
  local("help", "keys and commands"),
  local("quit", "close the window; the session keeps running"),
];

/** Whether the text is a command being typed: a slash, and no space or newline yet. */
export function isSlashWord(text: string): boolean {
  return text.startsWith("/") && !/\s/.test(text);
}

/**
 * The commands for what is typed after the slash: names starting with it, else (when none does)
 * names containing it.
 */
export function matching(typed: string): Command[] {
  const query = (typed.startsWith("/") ? typed.slice(1) : typed).toLowerCase();
  const starting = COMMANDS.filter((command) => command.name.startsWith(query));
  if (starting.length > 0) return starting;
  return COMMANDS.filter((command) => command.name.includes(query));
}

/** The command named by the first word of `text` ("/model sonnet" is model), if any. */
export function lookup(text: string): Command | undefined {
  const word = /^\s*\/(\S+)/.exec(text)?.[1];
  if (word === undefined) return undefined;
  return COMMANDS.find((command) => command.name === word);
}

/** What Tab or Enter puts in the input for a menu pick: the name and a space for arguments. */
export function completion(command: Command): string {
  return `/${command.name} `;
}

/** The highlighted row after moving by `delta`, wrapping around `count` rows. */
export function moveSelection(selected: number, delta: number, count: number): number {
  if (count <= 0) return 0;
  return (((selected + delta) % count) + count) % count;
}

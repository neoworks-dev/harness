// The slash commands the `/` menu lists, and how what is typed picks among them. Plain functions,
// so they are tested without a window. The daemon answers most commands itself (`/model` opens a
// choice card), and those are listed here; the ones the window handles come from plugins, through
// the `commands` service, with a `run`.

export interface Command {
  name: string;
  /** Argument hint shown after the name. */
  args: string;
  desc: string;
  /** Handles the command in this window instead of sending it to the daemon. */
  run?: () => void;
  /** Pointless without arguments: Enter completes it instead of running it. */
  needsArgs: boolean;
}

function daemon(name: string, args: string, desc: string, needsArgs = false): Command {
  return { name, args, desc, needsArgs };
}

/** The commands the daemon answers. */
export const COMMANDS: readonly Command[] = [
  daemon("clear", "", "start a new conversation in this directory"),
  daemon("cd", "[dir]", "move this session to another directory"),
  daemon("model", "[harness] [model]", "show or set the model"),
  daemon("effort", "[harness] [level]", "show or set the reasoning effort"),
  daemon("mode", "[harness] [mode]", "show or set the agent mode"),
  daemon("resume", "[claude|codex]", "continue an earlier Claude Code / Codex conversation"),
  daemon("claude", "<task>", "give a task to Claude Code", true),
  daemon("codex", "<task>", "give a task to Codex", true),
  daemon(
    "harness",
    "[neo|claude|codex]",
    "switch this session's agent: neo (based on pi), claude, codex",
  ),
  daemon("title", "", "write this session's title again, with a model"),
  daemon("title-model", "[harness] [model|default]", "show or set the model that writes titles"),
  daemon(
    "approvals",
    "[ask|review|edits|auto]",
    "ask, review each edit, accept edits, or auto (no value: next)",
  ),
  daemon("safesearch", "[strict|moderate|off]", "how much adult content web search may return"),
  daemon(
    "settings",
    "[toggle [on|off]]",
    "turn settings on or off, like Show, don't tell. (UI over text)",
  ),
  daemon("login", "[provider]", "add an API key or an account (ChatGPT, Copilot, Claude Pro/Max)"),
  daemon("logout", "[provider]", "remove a stored API key or account"),
  daemon("accounts", "", "list the API keys and accounts in use"),
  daemon(
    "plugin",
    "[install|enable|…]",
    "plugins for neo: list, search, install, uninstall, enable, disable, marketplace add",
  ),
];

/** Whether the text is a command being typed: a slash, and no space or newline yet. */
export function isSlashWord(text: string): boolean {
  return text.startsWith("/") && !/\s/.test(text);
}

/**
 * The commands for what is typed after the slash: names starting with it, else (when none does)
 * names containing it.
 */
export function matching(typed: string, commands: readonly Command[] = COMMANDS): Command[] {
  const query = (typed.startsWith("/") ? typed.slice(1) : typed).toLowerCase();
  const starting = commands.filter((command) => command.name.startsWith(query));
  if (starting.length > 0) return starting;
  return commands.filter((command) => command.name.includes(query));
}

/** The command named by the first word of `text` ("/model sonnet" is model), if any. */
export function lookup(text: string, commands: readonly Command[] = COMMANDS): Command | undefined {
  const word = /^\s*\/(\S+)/.exec(text)?.[1];
  if (word === undefined) return undefined;
  return commands.find((command) => command.name === word);
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

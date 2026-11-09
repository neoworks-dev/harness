// Opens a session in neo-term, in a new terminal window: $TERMINAL when set, else the first
// terminal found on PATH. Each takes the command to run its own way.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const TERMINALS: Record<string, (command: string[]) => string[]> = {
  "xdg-terminal-exec": (command) => command,
  ghostty: (command) => ["-e", ...command],
  kitty: (command) => command,
  foot: (command) => command,
  alacritty: (command) => ["-e", ...command],
  wezterm: (command) => ["start", "--", ...command],
  konsole: (command) => ["-e", ...command],
  "gnome-terminal": (command) => ["--", ...command],
};

/** The argv that runs `command` in `terminal`, which is a name or a path to one. */
export function terminalArgv(terminal: string, command: string[]): string[] {
  const name = terminal.split("/").at(-1) ?? terminal;
  const argsOf = TERMINALS[name] ?? ((run: string[]) => ["-e", ...run]);
  return [terminal, ...argsOf(command)];
}

export function findTerminal(
  env: NodeJS.ProcessEnv,
  onPath: (name: string) => boolean,
): string | undefined {
  if (env.TERMINAL !== undefined && env.TERMINAL !== "") return env.TERMINAL;
  return Object.keys(TERMINALS).find(onPath);
}

/** neo-term on PATH, else the one this checkout builds into bin/. */
export function neoTermPath(onPath: (name: string) => boolean): string {
  if (onPath("neo-term")) return "neo-term";
  const built = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "bin", "neo-term");
  return existsSync(built) ? built : "neo-term";
}

export function openInTerm(session: string, cwd: string): void {
  const terminal = findTerminal(process.env, isOnPath);
  if (terminal === undefined) throw new Error("no terminal found: set $TERMINAL");
  const [program = terminal, ...args] = terminalArgv(terminal, [
    neoTermPath(isOnPath),
    "--session",
    session,
  ]);
  const child = spawn(program, args, {
    cwd: existsSync(cwd) ? cwd : undefined,
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}

function isOnPath(name: string): boolean {
  return (process.env.PATH ?? "").split(":").some((directory) => existsSync(join(directory, name)));
}

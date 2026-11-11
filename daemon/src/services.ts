// Background services the daemon starts with itself (the lsp and tools services), so their tools
// work before any app opens. Each registers like an app over the bus; the daemon only owns the
// process. NEO_SERVICES=none starts nothing. Services get NEO_DAEMON_PID and exit when that
// process is gone, so a killed daemon doesn't leave them behind. Which ones run follows what
// the user does with neo (see servicesFor): the language servers are for coding.
import { join } from "node:path";
import type { Subprocess } from "bun";
import type { Logger } from "./log.ts";
import type { Work } from "@neoworks-dev/neo-protocol/types";

const LSP: string[] = [
  process.execPath,
  "run",
  join(import.meta.dir, "..", "..", "services", "lsp", "main.ts"),
];

const TOOLS: string[] = [
  process.execPath,
  "run",
  join(import.meta.dir, "..", "..", "services", "tools", "main.ts"),
];

export const DEFAULT_SERVICES: string[][] = [LSP];

/**
 * The services for a kind of work: the tools the agent wrote for itself whatever the work, and
 * the language servers for coding.
 */
export function servicesFor(work: Work): string[][] {
  const services: string[][] = [TOOLS];
  if (work === "coding") services.push(...DEFAULT_SERVICES);
  return services;
}

export interface Services {
  /** Runs these and only these: starts the new ones, stops the ones not listed any more. */
  set(commands: string[][]): void;
  stop(): Promise<void>;
}

async function end(proc: Subprocess): Promise<void> {
  proc.kill("SIGTERM");
  await Promise.race([proc.exited, Bun.sleep(3000)]);
  if (proc.exitCode === null) proc.kill("SIGKILL");
}

export function startServices(
  commands: string[][],
  env: Record<string, string | undefined>,
  log: Logger,
): Services {
  const procs = new Map<string, Subprocess>();
  const start = (command: string[]): void => {
    try {
      const proc = Bun.spawn(command, {
        env,
        stdin: "ignore",
        stdout: "ignore",
        stderr: "inherit",
      });
      procs.set(command.join("\0"), proc);
      void proc.exited.then((code) => {
        if (code !== 0 && code !== null) log.warn(`service ${command.join(" ")} exited (${code})`);
      });
    } catch (err) {
      log.warn(`can't start service ${command.join(" ")}: ${String(err)}`);
    }
  };
  const services: Services = {
    set(next) {
      const keys = new Set(next.map((command) => command.join("\0")));
      for (const [key, proc] of procs) {
        if (keys.has(key)) continue;
        procs.delete(key);
        void end(proc);
      }
      for (const command of next) if (!procs.has(command.join("\0"))) start(command);
    },
    async stop() {
      const all = [...procs.values()];
      procs.clear();
      await Promise.all(all.map(end));
    },
  };
  services.set(commands);
  return services;
}

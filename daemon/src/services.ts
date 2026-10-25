// Background services the daemon starts with itself (today: the lsp service), so their tools
// work before any app opens. Each registers like an app over the bus; the daemon only owns the
// process. NEO_SERVICES=none starts nothing. Services get NEO_DAEMON_PID and exit when that
// process is gone, so a killed daemon doesn't leave them behind.
import { join } from "node:path";
import type { Subprocess } from "bun";
import type { Logger } from "./log.ts";

export const DEFAULT_SERVICES: string[][] = [
  [process.execPath, "run", join(import.meta.dir, "..", "..", "services", "lsp", "main.ts")],
];

export interface Services {
  stop(): Promise<void>;
}

export function startServices(
  commands: string[][],
  env: Record<string, string | undefined>,
  log: Logger,
): Services {
  const procs: Subprocess[] = [];
  for (const command of commands) {
    try {
      const proc = Bun.spawn(command, {
        env,
        stdin: "ignore",
        stdout: "ignore",
        stderr: "inherit",
      });
      procs.push(proc);
      void proc.exited.then((code) => {
        if (code !== 0 && code !== null) log.warn(`service ${command.join(" ")} exited (${code})`);
      });
    } catch (err) {
      log.warn(`can't start service ${command.join(" ")}: ${String(err)}`);
    }
  }
  return {
    async stop() {
      for (const proc of procs) proc.kill("SIGTERM");
      await Promise.all(procs.map((p) => Promise.race([p.exited, Bun.sleep(3000)])));
      for (const proc of procs) if (proc.exitCode === null) proc.kill("SIGKILL");
    },
  };
}

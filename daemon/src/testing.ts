// A real daemon in a temp dir, for tests in any package (SDKs, e2e). Uses the offline echo agent
// unless another agent is given. Clients find it through `env` (SUITE_SOCKET etc.).
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Agent } from "@suite/harness/agent";
import { createDaemon, type Daemon } from "./daemon.ts";
import { createEchoAgent } from "./echo-agent.ts";
import { silentLogger } from "./log.ts";
import { resolvePaths } from "./paths.ts";

export interface TestDaemon extends Daemon {
  /** Env vars pointing clients at this daemon (SUITE_SOCKET, SUITE_DATA_DIR). */
  env: Record<string, string>;
  dir: string;
}

// Temp dirs outlive a daemon (restart tests reuse them) and bun test fires no exit hooks, so each
// dir is named after its test process and later runs remove those of processes that are gone.
const PREFIX = "suite-test-";

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

for (const name of readdirSync(tmpdir())) {
  const pid = Number(name.slice(PREFIX.length).split("-")[0]);
  if (name.startsWith(PREFIX) && pid > 0 && pid !== process.pid && !alive(pid)) {
    rmSync(join(tmpdir(), name), { recursive: true, force: true });
  }
}

export async function startTestDaemon(
  options: { agent?: Agent; dir?: string } = {},
): Promise<TestDaemon> {
  const dir = options.dir ?? mkdtempSync(join(tmpdir(), `${PREFIX}${process.pid}-`));
  const env = { SUITE_SOCKET: join(dir, "bus.sock"), SUITE_DATA_DIR: join(dir, "data") };
  const daemon = await createDaemon({
    paths: resolvePaths(env),
    agent: options.agent ?? createEchoAgent(),
    log: silentLogger,
    port: 0,
    defaultCwd: dir,
  });
  return { ...daemon, env, dir };
}

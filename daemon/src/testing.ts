// A real daemon in a temp dir, for tests in any package (SDKs, e2e). Uses the offline echo agent
// unless another agent is given. Clients find it through `env` (NEO_SOCKET etc.).
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Agent } from "@neoworks-dev/neo/agent";
import { createDaemon, type Daemon, type DaemonOptions } from "./daemon.ts";
import type { GuestConfig } from "./guests/guests.ts";
import { createEchoAgent } from "./echo-agent.ts";
import { type Logger, silentLogger } from "./log.ts";
import { resolvePaths } from "./paths.ts";

export interface TestDaemon extends Daemon {
  /** Env vars pointing clients at this daemon (NEO_SOCKET, NEO_DATA_DIR). */
  env: Record<string, string>;
  dir: string;
}

// Temp dirs outlive a daemon (restart tests reuse them) and bun test fires no exit hooks, so each
// dir is named after its test process and later runs remove those of processes that are gone.
const PREFIX = "neo-test-";

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
  options: {
    agent?: Agent;
    dir?: string;
    guests?: Record<string, GuestConfig>;
    /** The neo that delegate {agent: "neo"} uses, when the agent isn't neo. */
    neo?: Agent;
    /** Default: a file in `dir`'s data directory and no pi file. */
    accounts?: { authPath?: string; piAuthPath?: string };
    /** DaemonOptions.oauthProviders. */
    oauthProviders?: DaemonOptions["oauthProviders"];
    /** Default: silent. */
    log?: Logger;
    /** A model writes titles after a first message: off unless a test is about them. */
    titles?: boolean;
    settingsTimeoutMs?: number;
    /** The MCP servers the daemon runs (DaemonOptions.mcp). */
    mcp?: DaemonOptions["mcp"];
    /** Where plugins are installed (DaemonOptions.plugins). */
    plugins?: string;
  } = {},
): Promise<TestDaemon> {
  const dir = options.dir ?? mkdtempSync(join(tmpdir(), `${PREFIX}${process.pid}-`));
  const env = { NEO_SOCKET: join(dir, "bus.sock"), NEO_DATA_DIR: join(dir, "data") };
  const daemon = await createDaemon({
    paths: resolvePaths(env),
    agent: options.agent ?? createEchoAgent(),
    log: options.log ?? silentLogger,
    port: 0,
    defaultCwd: dir,
    toolsDir: join(dir, "tools"),
    guests: options.guests,
    neo: options.neo,
    accounts: options.accounts,
    oauthProviders: options.oauthProviders,
    titles: options.titles ?? false,
    settingsTimeoutMs: options.settingsTimeoutMs,
    mcp: options.mcp,
    plugins: options.plugins,
  });
  return { ...daemon, env, dir };
}

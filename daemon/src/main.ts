// Daemon entry point: `bun run daemon` (or `bun run daemon/src/main.ts`).
//
// Env: SUITE_SOCKET, SUITE_DATA_DIR (see paths.ts); SUITE_AGENT=pi|echo (default pi); model and
// API key settings are read by the harness (see harness/src/config.ts).
import { mkdirSync } from "node:fs";
import { homedir } from "node:os";
import type { Agent } from "@suite/harness/agent";
import { createEchoAgent } from "./echo-agent.ts";
import { Hub } from "./hub.ts";
import { createLogger, type Logger } from "./log.ts";
import { resolvePaths } from "./paths.ts";
import { startServer } from "./server.ts";
import { SessionManager } from "./sessions.ts";
import { SessionStore } from "./store.ts";

const VERSION = "0.1.0";

async function createAgent(name: string, log: Logger): Promise<Agent> {
  if (name === "echo") return createEchoAgent();
  if (name !== "pi") throw new Error(`unknown SUITE_AGENT ${name} (expected pi or echo)`);
  const { createPiAgent } = await import("@suite/harness/pi");
  const { loadHarnessConfig } = await import("@suite/harness/config");
  const config = loadHarnessConfig();
  log.info(`agent: pi, model ${config.provider}/${config.modelId}`);
  return createPiAgent({ config, logger: log });
}

async function main(): Promise<void> {
  const log = createLogger();
  const paths = resolvePaths();
  mkdirSync(paths.dataDir, { recursive: true, mode: 0o700 });

  const store = new SessionStore(paths.db);
  const agent = await createAgent(process.env.SUITE_AGENT ?? "pi", log);
  const sessions = new SessionManager(store, agent, { defaultCwd: homedir(), log });
  sessions.recoverInterruptedTurns();

  const hub = new Hub(
    store,
    sessions,
    { version: VERSION, pid: process.pid, startedAt: Date.now() },
    log,
  );
  const server = await startServer(hub, paths.socket);
  log.info(`listening on ${paths.socket}, data in ${paths.dataDir}`);

  let stopping = false;
  const stop = async (signal: string): Promise<void> => {
    if (stopping) return;
    stopping = true;
    log.info(`${signal}: shutting down`);
    await server.stop();
    await sessions.shutdown(2000);
    store.close();
    process.exit(0);
  };
  process.on("SIGINT", () => void stop("SIGINT"));
  process.on("SIGTERM", () => void stop("SIGTERM"));
}

try {
  await main();
} catch (err) {
  process.stderr.write(`suite daemon: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
}

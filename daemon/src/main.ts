// Daemon entry point: `bun run daemon` (or `bun run daemon/src/main.ts`).
//
// Env: SUITE_SOCKET, SUITE_DATA_DIR (see paths.ts), SUITE_PORT (TCP, default: any free port);
// SUITE_AGENT=neo|echo (default neo); SUITE_SERVICES=none skips the background services (lsp);
// SUITE_TITLES=off keeps a model from writing session titles (/title still does, see titles.ts);
// guests (Claude Code, Codex) are configured in guests/config.ts (SUITE_GUESTS=none: none);
// model and API key settings are read by the harness (see harness/src/config.ts).
import { homedir } from "node:os";
import { join } from "node:path";
import type { Agent } from "@suite/harness/agent";
import { authPaths } from "@suite/harness/config";
import { suiteConfigDir } from "@suite/harness/mcp-config";
import { createDaemon } from "./daemon.ts";
import { createEchoAgent } from "./echo-agent.ts";
import { loadGuestConfigs } from "./guests/config.ts";
import { createLogger, type Logger } from "./log.ts";
import { resolvePaths } from "./paths.ts";
import { DEFAULT_SERVICES, startServices } from "./services.ts";
import { loadTheme } from "./theme.ts";

async function createAgent(name: string, log: Logger): Promise<Agent> {
  if (name === "echo") return createEchoAgent();
  if (name !== "neo") throw new Error(`unknown SUITE_AGENT ${name} (expected neo or echo)`);
  const { createNeoAgent } = await import("@suite/harness/neo");
  const { loadHarnessConfig } = await import("@suite/harness/config");
  const config = loadHarnessConfig();
  log.info(`agent: neo, model ${config.provider}/${config.modelId}`);
  return createNeoAgent({ config, logger: log });
}

async function main(): Promise<void> {
  const log = createLogger();
  const paths = resolvePaths();
  const daemon = await createDaemon({
    paths,
    agent: await createAgent(process.env.SUITE_AGENT ?? "neo", log),
    log,
    port: Number(process.env.SUITE_PORT ?? 0),
    defaultCwd: homedir(),
    theme: loadTheme(process.env, log),
    guests: loadGuestConfigs(),
    titles: process.env.SUITE_TITLES !== "off",
    accounts: authPaths(),
    mcp: {
      configFile: join(suiteConfigDir(), "config.json"),
      plugins: join(suiteConfigDir(), "plugins"),
    },
  });
  log.info(`listening on ${paths.socket} and ${daemon.info.url}, data in ${paths.dataDir}`);
  const commands: string[][] = [];
  if (process.env.SUITE_SERVICES !== "none") commands.push(...DEFAULT_SERVICES);
  const services = startServices(
    commands,
    {
      ...process.env,
      SUITE_SOCKET: paths.socket,
      SUITE_DAEMON_PID: String(process.pid),
    },
    log,
  );

  let stopping = false;
  const stop = async (signal: string): Promise<void> => {
    if (stopping) return;
    stopping = true;
    log.info(`${signal}: shutting down`);
    await services.stop();
    await daemon.stop();
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

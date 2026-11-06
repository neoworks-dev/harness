// Daemon entry point: `bun run daemon` (or `bun run daemon/src/main.ts`).
//
// Env: NEO_SOCKET, NEO_DATA_DIR (see paths.ts), NEO_PORT (TCP, default: any free port);
// NEO_AGENT=neo|echo (default neo); NEO_SERVICES=none skips the background services (lsp);
// NEO_TITLES=off keeps a model from writing session titles (/title still does, see titles.ts);
// guests (Claude Code, Codex) are configured in guests/config.ts (NEO_GUESTS=none: none);
// model and API key settings are read by the harness (see neo/src/config.ts).
import { homedir } from "node:os";
import { join } from "node:path";
import type { Agent } from "@neoworks-dev/neo/agent";
import { authPaths } from "@neoworks-dev/neo/config";
import { neoConfigDir } from "@neoworks-dev/neo/mcp-config";
import { createDaemon } from "./daemon.ts";
import { createEchoAgent } from "./echo-agent.ts";
import { loadGuestConfigs } from "./guests/config.ts";
import { createLogger, type Logger } from "./log.ts";
import { resolvePaths } from "./paths.ts";
import { servicesFor, startServices } from "./services.ts";
import { loadTheme } from "./theme.ts";

async function createAgent(name: string, log: Logger): Promise<Agent> {
  if (name === "echo") return createEchoAgent();
  if (name !== "neo") throw new Error(`unknown NEO_AGENT ${name} (expected neo or echo)`);
  const { createNeoAgent } = await import("@neoworks-dev/neo/neo");
  const { loadHarnessConfig } = await import("@neoworks-dev/neo/config");
  const config = loadHarnessConfig();
  log.info(`agent: neo, model ${config.provider}/${config.modelId}`);
  return createNeoAgent({ config, logger: log });
}

async function main(): Promise<void> {
  const log = createLogger();
  const paths = resolvePaths();
  const daemon = await createDaemon({
    paths,
    agent: await createAgent(process.env.NEO_AGENT ?? "neo", log),
    log,
    port: Number(process.env.NEO_PORT ?? 0),
    defaultCwd: homedir(),
    theme: loadTheme(process.env, log),
    guests: loadGuestConfigs(),
    titles: process.env.NEO_TITLES !== "off",
    accounts: authPaths(),
    plugins: join(neoConfigDir(), "plugins"),
    mcp: { configFile: join(neoConfigDir(), "config.json") },
  });
  log.info(`listening on ${paths.socket} and ${daemon.info.url}, data in ${paths.dataDir}`);
  const wanted = (): string[][] => {
    const commands: string[][] = [];
    if (process.env.NEO_SERVICES !== "none") {
      commands.push(...servicesFor(daemon.settings.work() ?? "coding"));
    }
    return commands;
  };
  const services = startServices(
    wanted(),
    {
      ...process.env,
      NEO_SOCKET: paths.socket,
      NEO_DAEMON_PID: String(process.pid),
    },
    log,
  );
  // What the user does with neo decides which services run.
  daemon.settings.onToggle(() => services.set(wanted()));

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
  process.stderr.write(`neo daemon: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
}

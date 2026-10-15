// Wires the daemon's parts together. Used by main.ts and by tests (startTestDaemon).
import { mkdirSync } from "node:fs";
import type { Agent } from "@suite/harness/agent";
import { Hub, PROTOCOL_VERSION } from "./hub.ts";
import type { Logger } from "./log.ts";
import type { DaemonPaths } from "./paths.ts";
import { type BusServer, type DaemonInfoFile, startServer } from "./server.ts";
import { SessionManager } from "./sessions.ts";
import { SessionStore } from "./store.ts";

export const VERSION = "0.2.0";

export interface DaemonOptions {
  paths: DaemonPaths;
  agent: Agent;
  log: Logger;
  /** TCP port on 127.0.0.1; 0 picks a free one. */
  port: number;
  defaultCwd: string;
}

export interface Daemon {
  info: DaemonInfoFile;
  store: SessionStore;
  sessions: SessionManager;
  hub: Hub;
  /** Cancels running turns (waiting up to timeoutMs), closes listeners and the database. */
  stop(timeoutMs?: number): Promise<void>;
}

export async function createDaemon(options: DaemonOptions): Promise<Daemon> {
  const { paths, agent, log } = options;
  mkdirSync(paths.dataDir, { recursive: true, mode: 0o700 });
  const store = new SessionStore(paths.db);
  const sessions = new SessionManager(store, agent, { defaultCwd: options.defaultCwd, log });
  sessions.recoverInterruptedTurns();
  const hub = new Hub(
    store,
    sessions,
    { version: VERSION, pid: process.pid, startedAt: Date.now() },
    log,
  );
  let server: BusServer;
  try {
    server = await startServer(hub, { paths, port: options.port, protocol: PROTOCOL_VERSION });
  } catch (err) {
    store.close();
    throw err;
  }
  return {
    info: server.info,
    store,
    sessions,
    hub,
    async stop(timeoutMs = 2000) {
      await server.stop();
      await sessions.shutdown(timeoutMs);
      store.close();
    },
  };
}

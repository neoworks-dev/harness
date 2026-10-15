// Wires the daemon's parts together. Used by main.ts and by tests (startTestDaemon).
import { mkdirSync } from "node:fs";
import type { Agent } from "@suite/harness/agent";
import type { Theme } from "@suite/protocol/types";
import defaultTheme from "../../protocol/themes/default.json";
import { ActionRunner } from "./actions.ts";
import { FocusManager } from "./focus.ts";
import { ViewManager } from "./view/views.ts";
import { Gateway } from "./gateway/gateway.ts";
import { mcpHandler } from "./gateway/mcp-server.ts";
import { ConnectorRegistry } from "./gateway/registry.ts";
import { type ConnectUpstream, connectMcp } from "./gateway/upstream.ts";
import { Hub, PROTOCOL_VERSION } from "./hub.ts";
import type { Logger } from "./log.ts";
import type { DaemonPaths } from "./paths.ts";
import { type BusServer, type DaemonInfoFile, startServer } from "./server.ts";
import { SessionManager } from "./sessions.ts";
import { SessionStore } from "./store.ts";

import { VERSION } from "./version.ts";

export interface DaemonOptions {
  paths: DaemonPaths;
  agent: Agent;
  log: Logger;
  /** TCP port on 127.0.0.1; 0 picks a free one. */
  port: number;
  defaultCwd: string;
  /** Served to every client; the built-in default when absent. */
  theme?: Theme;
  /** Replaces the MCP client used to reach apps; for tests. */
  connectUpstream?: ConnectUpstream;
}

export interface Daemon {
  info: DaemonInfoFile;
  store: SessionStore;
  sessions: SessionManager;
  hub: Hub;
  connectors: ConnectorRegistry;
  gateway: Gateway;
  focus: FocusManager;
  views: ViewManager;
  /** Cancels running turns (waiting up to timeoutMs), closes listeners and the database. */
  stop(timeoutMs?: number): Promise<void>;
}

export async function createDaemon(options: DaemonOptions): Promise<Daemon> {
  const { paths, agent, log } = options;
  mkdirSync(paths.dataDir, { recursive: true, mode: 0o700 });
  const store = new SessionStore(paths.db);
  const connectors = new ConnectorRegistry(store, options.connectUpstream ?? connectMcp, log);
  let focus: FocusManager | undefined;
  const gateway = new Gateway(connectors, {
    log,
    focus: (app, session) => {
      try {
        focus?.move({ app, session }, "agent");
        return undefined;
      } catch (err) {
        return err instanceof Error ? err.message : String(err);
      }
    },
  });
  let server: BusServer | undefined;
  const sessions = new SessionManager(store, agent, {
    defaultCwd: options.defaultCwd,
    log,
    connectors: () => connectors.snapshot(),
    focus: (sessionId) => focus?.focusOf(sessionId),
    mcpServers: (sessionId) => {
      if (!server) return [];
      const { url, token } = server.info;
      return [
        {
          name: "suite",
          url: `${url}/mcp`,
          headers: [
            { name: "authorization", value: `Bearer ${token}` },
            { name: "x-suite-session", value: sessionId },
          ],
        },
      ];
    },
  });
  sessions.recoverInterruptedTurns();
  focus = new FocusManager(store, sessions, connectors, log);
  const views = new ViewManager(store, (tool) => connectors.template(tool), log);
  const actions = new ActionRunner({ store, sessions, connectors, gateway, views });
  const hub = new Hub(
    store,
    sessions,
    { version: VERSION, pid: process.pid, startedAt: Date.now() },
    log,
    {
      connectors,
      focus,
      views,
      theme: options.theme ?? defaultTheme,
      action: (action, from) => actions.run(action, from),
    },
  );
  try {
    server = await startServer(hub, {
      paths,
      port: options.port,
      protocol: PROTOCOL_VERSION,
      mcp: mcpHandler(gateway),
    });
  } catch (err) {
    store.close();
    throw err;
  }
  const started = server;
  return {
    info: started.info,
    store,
    sessions,
    hub,
    connectors,
    gateway,
    focus,
    views,
    async stop(timeoutMs = 2000) {
      await started.stop();
      await sessions.shutdown(timeoutMs);
      await connectors.close();
      store.close();
    },
  };
}

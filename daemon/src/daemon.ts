// Wires the daemon's parts together. Used by main.ts and by tests (startTestDaemon).
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Agent, McpServer } from "@suite/harness/agent";
import type { Theme } from "@suite/protocol/types";
import defaultTheme from "../../protocol/themes/default.json";
import { ActionRunner } from "./actions.ts";
import { AssetStore } from "./assets.ts";
import { FocusManager } from "./focus.ts";
import { ViewManager } from "./view/views.ts";
import { Gateway } from "./gateway/gateway.ts";
import { mcpHandler } from "./gateway/mcp-server.ts";
import { ConnectorRegistry } from "./gateway/registry.ts";
import { createDelegate } from "./guests/delegate.ts";
import { type GuestConfig, GuestManager } from "./guests/guests.ts";
import { Asker } from "./asker.ts";
import { Commands } from "./commands.ts";
import { Settings } from "./settings.ts";
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
  /** Guest agents (see guests/config.ts); none when absent. */
  guests?: Record<string, GuestConfig>;
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
  assets: AssetStore;
  guests: GuestManager;
  actions: ActionRunner;
  commands: Commands;
  settings: Settings;
  /** Cancels running turns (waiting up to timeoutMs), closes listeners and the database. */
  stop(timeoutMs?: number): Promise<void>;
}

export async function createDaemon(options: DaemonOptions): Promise<Daemon> {
  const { paths, agent, log } = options;
  mkdirSync(paths.dataDir, { recursive: true, mode: 0o700 });
  const store = new SessionStore(paths.db);
  const assets = new AssetStore(join(paths.dataDir, "assets"));
  const connectors = new ConnectorRegistry(store, options.connectUpstream ?? connectMcp, log);
  let focus: FocusManager | undefined;
  let server: BusServer | undefined;
  /** The gateway for a session's agent, or for one of its guests (which then gets refs). */
  const mcpServers = (sessionId: string, guest?: string): McpServer[] => {
    if (!server) return [];
    const { url, token } = server.info;
    const headers = [
      { name: "authorization", value: `Bearer ${token}` },
      { name: "x-suite-session", value: sessionId },
    ];
    if (guest !== undefined) headers.push({ name: "x-suite-guest", value: guest });
    return [{ name: "suite", url: `${url}/mcp`, headers }];
  };
  const settings = new Settings(store);
  const asker = new Asker((session, event) => {
    store.append(session, event);
  });
  const guests = new GuestManager({
    configs: options.guests ?? {},
    log,
    append: (session, event) => {
      store.append(session, event);
    },
    cwd: (session) => store.getSession(session)?.cwd ?? options.defaultCwd,
    mcpServers,
    connectors: () => connectors.snapshot(),
    focus: (session) => focus?.focusOf(session),
    settings: (session, guest) => settings.get(session, guest),
    asker,
  });
  const gateway = new Gateway(connectors, {
    log,
    resolveAsset: (handle) => assets.resolve(handle),
    recordResult: (session, result) => {
      store.append(session, result);
    },
    delegate: createDelegate({
      guests,
      asker,
      settings: (session, guest) => settings.get(session, guest),
      append: (session, event) => {
        store.append(session, event);
      },
      main: agent.name,
    }),
    focus: (app, session) => {
      try {
        focus?.move({ app, session }, "agent");
        return undefined;
      } catch (err) {
        return err instanceof Error ? err.message : String(err);
      }
    },
  });
  const sessions = new SessionManager(store, agent, {
    defaultCwd: options.defaultCwd,
    resolveAsset: (handle) => assets.resolve(handle),
    log,
    connectors: () => connectors.snapshot(),
    focus: (sessionId) => focus?.focusOf(sessionId),
    mcpServers: (sessionId) => mcpServers(sessionId),
    guest: (sessionId, name) => guests.turnSession(sessionId, name),
    settings: (sessionId) => settings.get(sessionId, agent.name),
  });
  sessions.recoverInterruptedTurns();
  focus = new FocusManager(store, sessions, connectors, log);
  const views = new ViewManager(store, (tool) => connectors.template(tool), log);
  const actions = new ActionRunner({ store, sessions, connectors, gateway, views, asker });
  const commands = new Commands({ sessions, guests, settings, asker, log, main: agent.name });
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
      assets,
      action: (action, from) => actions.run(action, from),
      command: (session, text) => commands.run(session, text),
    },
  );
  // The agent's write/edit results carry diffs; each one means a file changed on disk.
  store.onAppend(({ session, event }) => {
    if (event.type !== "tool_call_update") return;
    for (const content of event.content ?? []) {
      if (content.type === "diff") hub.fileChanged({ path: content.path, app: "agent", session });
    }
  });
  try {
    server = await startServer(hub, {
      paths,
      port: options.port,
      protocol: PROTOCOL_VERSION,
      mcp: mcpHandler(gateway),
      assets,
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
    assets,
    guests,
    actions,
    commands,
    settings,
    async stop(timeoutMs = 2000) {
      await started.stop();
      await sessions.shutdown(timeoutMs);
      asker.dropAll();
      guests.shutdown();
      await connectors.close();
      store.close();
    },
  };
}

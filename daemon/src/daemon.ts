// Wires the daemon's parts together. Used by main.ts and by tests (startTestDaemon).
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { findEnvKeys, getProviders } from "@mariozechner/pi-ai";
import { getOAuthProviders } from "@mariozechner/pi-ai/oauth";
import type { Agent, McpServer } from "@neoworks-dev/neo/agent";
import type { SlashCommand, Theme } from "@neoworks-dev/neo-protocol/types";
import defaultTheme from "../../protocol/themes/default.json";
import { ActionRunner, openUiTools } from "./actions.ts";
import { Todos, todosTool } from "./todos.ts";
import { AssetStore } from "./assets.ts";
import { FocusManager } from "./focus.ts";
import { ViewManager } from "./view/views.ts";
import { Gateway } from "./gateway/gateway.ts";
import { mcpHandler } from "./gateway/mcp-server.ts";
import { McpServers, type McpServersOptions } from "./gateway/mcp-servers.ts";
import { ConnectorRegistry } from "./gateway/registry.ts";
import { uiLibraryText } from "./gateway/ui-library.ts";
import { createGuestAccounts } from "./guests/accounts.ts";
import { createDelegate } from "./guests/delegate.ts";
import { type GatewayRole, type GuestConfig, GuestManager } from "./guests/guests.ts";
import { loadMcpServers, projectMcpAllowed } from "@neoworks-dev/neo/mcp-config";
import {
  loadPluginCommands,
  type PluginCommand,
  pluginPrompt,
} from "@neoworks-dev/neo/plugin-commands";
import { Asker } from "./asker.ts";
import { Commands, DAEMON_COMMANDS } from "./commands.ts";
import { PluginCommands } from "./plugins.ts";
import { Login } from "./login.ts";
import { MESSAGES_APP, Messenger } from "./messages.ts";
import { handoffSummary } from "./handoff.ts";
import { Permissions } from "./permissions.ts";
import { Review } from "./review.ts";
import { UiLint } from "./ui-lint.ts";
import { Settings } from "./settings.ts";
import { SessionSummaries } from "./summaries.ts";
import { type ConnectUpstream, connectMcp } from "./gateway/upstream.ts";
import { Hub, PROTOCOL_VERSION } from "./hub.ts";
import type { Logger } from "./log.ts";
import type { DaemonPaths } from "./paths.ts";
import { type BusServer, type DaemonInfoFile, startServer } from "./server.ts";
import { SessionManager } from "./sessions.ts";
import { SessionStore } from "./store.ts";
import type { ShowDontTellSetting } from "./told.ts";
import { Titles } from "./titles.ts";

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
  /**
   * The neo harness delegate hands tasks to ("neo" subagents are new sessions of it). Defaults to
   * `agent` when that is neo; tests give one with a fake model.
   */
  neo?: Agent;
  /** Whether a model writes a session's title after its first message (default: yes). */
  titles?: boolean;
  /** How long a harness gets to tell its options for session.settings; for tests. */
  settingsTimeoutMs?: number;
  /**
   * Where /login keeps accounts (`authPath`, written) and where pi's are (`piAuthPath`, read
   * only). Default: auth.json in the data dir and no pi file; main.ts passes the harness's paths.
   */
  accounts?: { authPath?: string; piAuthPath?: string };
  /**
   * Where plugins are installed (neo/src/plugins.ts): /plugin works on it, and the enabled
   * plugins' commands, MCP servers and language servers are used. No plugins without.
   */
  plugins?: string;
  /**
   * The MCP servers the daemon runs (see neo/src/mcp-config.ts): config.json, the enabled
   * plugins' `.mcp.json`, and the sessions' directories when config.json allows. None without.
   */
  mcp?: {
    configFile?: string;
    env?: Record<string, string | undefined>;
    servers?: McpServersOptions;
  };
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
  titles: Titles;
  /** The MCP servers it runs; `reload` reads their configuration again (plugins changed). */
  mcp: { servers: McpServers; reload(): Promise<void> };
  /** Cancels running turns (waiting up to timeoutMs), closes listeners and the database. */
  stop(timeoutMs?: number): Promise<void>;
}

export async function createDaemon(options: DaemonOptions): Promise<Daemon> {
  const { paths, agent, log } = options;
  mkdirSync(paths.dataDir, { recursive: true, mode: 0o700 });
  const store = new SessionStore(paths.db);
  const assets = new AssetStore(join(paths.dataDir, "assets"));
  const connectors = new ConnectorRegistry(
    store,
    (endpoint) => stdioServers.connect(options.connectUpstream ?? connectMcp)(endpoint),
    log,
  );
  const stdioServers = new McpServers(connectors, log, options.mcp?.servers);
  /** Reads the MCP configuration and makes it the servers that run; one read at a time. */
  let mcpLoading: Promise<void> = Promise.resolve();
  const reloadMcp = (): Promise<void> => {
    if (!options.mcp && !options.plugins) return Promise.resolve();
    const { configFile, env } = options.mcp ?? {};
    const { plugins } = options;
    const run = async (): Promise<void> => {
      const projects: string[] = [];
      if (projectMcpAllowed(configFile)) {
        projects.push(options.defaultCwd, ...store.listSessions().map((row) => row.cwd));
      }
      const loaded = loadMcpServers({ configFile, plugins, projects, env });
      for (const problem of loaded.problems) log.warn(`mcp: ${problem}`);
      await stdioServers.set(loaded.servers);
    };
    mcpLoading = mcpLoading.then(run, run);
    return mcpLoading;
  };
  void reloadMcp();
  let focus: FocusManager | undefined;
  let server: BusServer | undefined;
  /**
   * The gateway for a session's agent. `role` says who else calls: subagents may not delegate,
   * ACP clients (guests and the session's own ACP harness) get text-only results with refs.
   */
  const mcpServers = (sessionId: string, role?: GatewayRole): McpServer[] => {
    if (!server) return [];
    const { url, token } = server.info;
    const headers = [
      { name: "authorization", value: `Bearer ${token}` },
      { name: "x-neo-session", value: sessionId },
    ];
    if (role?.guest === true) headers.push({ name: "x-neo-guest", value: role.name });
    if (role?.text === true) headers.push({ name: "x-neo-text", value: "1" });
    return [{ name: "neo", url: `${url}/mcp`, headers }];
  };
  const settings = new Settings(store, agent.name);
  const asker = new Asker((session, event) => {
    store.append(session, event);
  });
  const permissions = new Permissions({
    asker,
    approvals: (session) => settings.approvals(session),
    cwd: (session) => store.getSession(session)?.cwd ?? options.defaultCwd,
    fileChanged: (path, session) => hub.fileChanged({ path, app: "agent", session }),
  });
  /** The harness a session runs on: its choice if that still exists, else the daemon's agent. */
  const harnessOf = (session: string): string => {
    const name = settings.harness(session);
    return name === agent.name || guests.has(name) ? name : agent.name;
  };
  // "Show, don't tell." (/settings): each agent is told with its next prompt, and given the UI
  // library as ui_library would return it then.
  const showDontTell = (): ShowDontTellSetting => ({
    on: settings.showDontTell(),
    library: () => uiLibraryText(connectors.componentsOf(), connectors.openApps()),
  });
  const authPath = options.accounts?.authPath ?? join(paths.dataDir, "auth.json");
  const piAuthPath = options.accounts?.piAuthPath ?? join(paths.dataDir, "pi-auth.json");
  const guests = new GuestManager({
    accounts: createGuestAccounts({ authPath, piAuthPath }),
    configs: options.guests ?? {},
    log,
    append: (session, event) => {
      store.append(session, event);
    },
    cwd: (session) => store.getSession(session)?.cwd ?? options.defaultCwd,
    mcpServers,
    connectors: () => connectors.snapshot(),
    focus: (session) => focus?.focusOf(session),
    showDontTell,
    settings: (session, guest) => settings.get(session, guest),
    asker,
    approvals: (session) => settings.approvals(session),
    guard: permissions.guard,
    handoff: (session, harness, all) => handoffSummary(store, session, harness, agent.name, all),
    storeImage: (data, mime) => assets.put({ bytes: Buffer.from(data, "base64"), mime }).handle,
    neo: options.neo ?? (agent.name === "neo" ? agent : undefined),
    resolveAsset: (handle) => assets.resolve(handle),
    requestPermission: (session, name) => permissions.hook(session, name),
    // Not for a delegation's own session: subagents don't start subagents.
    nativeSubagents: (session) =>
      settings.subagents() !== "delegate" && store.getSession(session)?.parent === undefined,
    alwaysAskForAgents: () => settings.alwaysAskForAgents(),
  });
  /** The names a plugin's command may not take: what the daemon answers itself, and the guests'. */
  const reservedCommands = (): Set<string> => new Set([...DAEMON_COMMANDS, ...guests.names()]);
  const pluginCommands = (): PluginCommand[] =>
    loadPluginCommands(options.plugins, reservedCommands());
  /** What each session was last told neo's commands are, as JSON. */
  const toldCommands = new Map<string, string>();
  /** Puts neo's commands (the enabled plugins') in the session's log when they have changed. */
  const announceCommands = (session: string): void => {
    if (!options.plugins || harnessOf(session) !== agent.name) return;
    const commands = pluginCommands().map((command): SlashCommand => {
      const out: SlashCommand = { name: command.name, description: command.description };
      if (command.hint !== undefined) out.hint = command.hint;
      return out;
    });
    const now = JSON.stringify(commands);
    let told = toldCommands.get(session);
    if (told === undefined) {
      told = "[]";
      for (const record of store.read(session, 1)) {
        const { event } = record;
        if (event.type === "commands_available" && event.harness === agent.name) {
          told = JSON.stringify(event.commands);
        }
      }
    }
    toldCommands.set(session, now);
    if (told !== now) {
      store.append(session, { type: "commands_available", harness: agent.name, commands });
    }
  };
  const summaries = new SessionSummaries(store);
  const messenger = new Messenger({
    append: (session, event) => {
      store.append(session, event);
    },
    wake: (session) => sessions.wake(session),
  });
  // Whatever the user writes gives agents a fresh allowance of messages.
  store.onAppend(({ session, event }) => {
    if (event.type !== "user_message") return;
    if (event.app !== MESSAGES_APP && event.app !== "delegate") messenger.newRound(session);
  });
  const sessions = new SessionManager(store, agent, {
    describe: (sessionId) => ({ ...summaries.get(sessionId), harness: harnessOf(sessionId) }),
    defaultCwd: options.defaultCwd,
    resolveAsset: (handle) => assets.resolve(handle),
    log,
    connectors: () => connectors.snapshot(),
    focus: (sessionId) => focus?.focusOf(sessionId),
    showDontTell,
    mcpServers: (sessionId) => mcpServers(sessionId),
    guest: (sessionId, name) => guests.turnSession(sessionId, name),
    harness: harnessOf,
    agents: (name) => (guests.has(name) ? guests.agent(name) : undefined),
    settings: (sessionId, harness) => settings.get(sessionId, harness),
    requestPermission: (sessionId) => permissions.hook(sessionId, agent.name),
    safeSearch: () => settings.safeSearch(),
    created: (sessionId) => {
      settings.inheritHarness(sessionId, (name) => name === agent.name || guests.has(name));
      // A new directory may bring its own .mcp.json.
      if (projectMcpAllowed(options.mcp?.configFile)) void reloadMcp();
      announceCommands(sessionId);
    },
    takeMessages: (sessionId) => messenger.take(sessionId, undefined),
    steer: () => settings.steer(),
    backgroundEnded: (sessionId, text) => messenger.tell(sessionId, "background", text),
  });
  // After the summaries, so the start of a first message names the session before a model does.
  const titles = new Titles({
    store,
    settings,
    log,
    main: agent.name,
    can: (harness) =>
      harness === agent.name ? agent.generateText !== undefined : guests.has(harness),
    generate: (harness, params) =>
      harness === agent.name && agent.generateText
        ? agent.generateText(params)
        : guests.generateText(harness, params),
    changed: (session) => sessions.changed(session),
    auto: options.titles ?? true,
  });
  sessions.recoverInterruptedTurns();
  // A deleted session takes what was kept for it along (views drop theirs once they exist).
  sessions.onRemove((session) => {
    summaries.forget(session);
    guests.forget(session);
    asker.forget(session);
    toldCommands.delete(session);
  });
  // A guest works where it was started: one that moved (/cd) starts its guests over.
  sessions.onMove((session) => guests.stop(session));
  // Sessions nobody wrote in are deleted once nothing follows them (see Hub); after a restart
  // nothing does yet. Before the focus is made, so it never starts on one that is gone.
  sessions.pruneEmpty();
  const delegate = createDelegate({
    guests,
    asker,
    sessions,
    store,
    messenger,
    settings: (session, guest) => settings.get(session, guest),
    append: (session, event) => {
      store.append(session, event);
    },
    main: harnessOf,
    approvals: (session) => settings.approvals(session),
    cwd: (session) => store.getSession(session)?.cwd ?? options.defaultCwd,
    autoApprove: (session) => settings.approvals(session) === "auto",
    alwaysAsk: () => settings.alwaysAskForAgents(),
    // Only an agent that has subagents of its own gives up delegate for them.
    nativeOnly: (session) => {
      const harness = harnessOf(session);
      return (
        settings.subagents() === "native" && guests.has(harness) && guests.ownSubagents(harness)
      );
    },
  });
  // The user cancelling a turn stops the session's background subagents with it.
  sessions.onCancel((session) => {
    delegate.stopBackground(session);
  });
  const todos = new Todos(store);
  const gateway = new Gateway(connectors, {
    log,
    // A subagent changes the list as itself, the session's agent as its harness.
    todos: (session, guest, args) => todosTool(todos, session, guest ?? harnessOf(session), args),
    resolveAsset: (handle) => assets.resolve(handle),
    recordResult: (session, result) => {
      store.append(session, result);
    },
    delegate: delegate.run,
    models: delegate.models,
    messages: messenger,
    focus: (app, session) => {
      try {
        focus?.move({ app, session }, "agent");
        return undefined;
      } catch (err) {
        return err instanceof Error ? err.message : String(err);
      }
    },
  });
  focus = new FocusManager(store, sessions, connectors, log);
  const views = new ViewManager(
    store,
    (tool) => connectors.template(tool),
    log,
    (session) => openUiTools({ store, gateway }, session),
    () => connectors.library(),
  );
  sessions.onRemove((session) => views.forget(session));
  const actions = new ActionRunner({
    store,
    sessions,
    connectors,
    gateway,
    views,
    asker,
    cancelAgent: delegate.cancel,
    todos,
  });
  const notice = (session: string, text: string, tone?: "muted" | "danger"): void => {
    store.append(
      session,
      tone === undefined ? { type: "notice", text } : { type: "notice", text, tone },
    );
  };
  const login = new Login({
    asker,
    log,
    notice,
    authPath,
    piAuthPath,
    oauthProviders: getOAuthProviders,
    apiProviders: getProviders,
    envKeys: (provider) => findEnvKeys(provider) ?? [],
  });
  const plugins = options.plugins;
  const pluginCommand =
    plugins === undefined
      ? undefined
      : new PluginCommands({
          dir: plugins,
          notice,
          changed: async (session) => {
            await reloadMcp();
            announceCommands(session);
          },
          cwd: (session) => store.getSession(session)?.cwd ?? options.defaultCwd,
        });
  const commands = new Commands({
    plugins: pluginCommand,
    sessions,
    guests,
    settings,
    titles,
    asker,
    login,
    log,
    notice,
    tell: (session, note, about) => {
      store.append(session, { type: "user_note", note, about });
    },
    main: agent.name,
    harness: harnessOf,
    harnesses: () => [agent.name, ...guests.names()],
    settingsTimeoutMs: options.settingsTimeoutMs,
  });
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
      userSettings: {
        get: () => settings.userSettings(),
        onChange: (listener) => settings.onToggle(listener),
      },
      assets,
      action: (action, from) => actions.run(action, from),
      command: (session, text, context) => commands.run(session, text, context),
      expand: (session, text) => {
        announceCommands(session);
        if (!options.plugins || harnessOf(session) !== agent.name) return undefined;
        return pluginPrompt(pluginCommands(), text);
      },
      settings: (session, known) => commands.settings(session, known),
    },
  );
  // The agent's write/edit results carry diffs; each one means a file changed on disk.
  store.onAppend(({ session, event }) => {
    if (event.type !== "tool_call_update") return;
    for (const content of event.content ?? []) {
      if (content.type === "diff") hub.fileChanged({ path: content.path, app: "agent", session });
    }
  });
  // In edits mode the turn's edits are already written; a card offers to put them back.
  new Review({
    store,
    asker,
    approvals: (session) => settings.approvals(session),
    fileChanged: (path, session) => hub.fileChanged({ path, app: "agent", session }),
    log,
  });
  // UI that ran long is told to the agent with its next prompt.
  new UiLint(store);
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
    titles,
    mcp: { servers: stdioServers, reload: reloadMcp },
    async stop(timeoutMs = 2000) {
      await started.stop();
      await sessions.shutdown(timeoutMs);
      await delegate.shutdown(timeoutMs);
      asker.dropAll();
      guests.shutdown();
      await mcpLoading.catch(() => undefined);
      await stdioServers.close();
      await connectors.close();
      hub.close();
      store.close();
    },
  };
}

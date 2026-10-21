// Connector registry: which connectors are open (presence over the bus), their MCP connections,
// and what we remember about every connector ever seen (tool schemas, version, launch command), so
// tools_search and "isn't open" errors work while an app is closed and after a daemon restart.
import { createHash } from "node:crypto";
import {
  type AppComponents,
  appLibrary,
  type AppLibrary,
  callNames,
  checkComponents,
} from "@neoworks/neoui";
import type {
  AppPresence,
  ConnectorInfo,
  ConnectorKind,
  McpEndpoint,
} from "@neoworks-dev/neo-protocol/types";
import type { Logger } from "../log.ts";
import type { SessionStore, StoredConnector, StoredTool } from "../store.ts";
import type { ConnectUpstream, Upstream } from "./upstream.ts";

/** An open connector as sessions are told about it. */
export interface ConnectorSnapshot {
  app: string;
  kind: ConnectorKind;
  v: number;
  /** Tool names without the app prefix. */
  tools: string[];
  /** What to write to use the components it offers: `photo-editor.BrightnessCard`. */
  components?: string[];
  /** One line for the agent: the presence's description, else the instructions' first sentence. */
  description?: string;
}

/** The first sentence (or line) of an MCP server's instructions, kept short. */
export function firstSentence(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  const end = line.search(/[.!?](\s|$)/);
  const sentence = end === -1 ? line : line.slice(0, end + 1);
  return sentence.length > 160 ? `${sentence.slice(0, 159)}…` : sentence;
}

interface Live {
  /** The bus client that registered it; only that client can unregister it. */
  owner: object;
  presence: AppPresence;
  upstream?: Upstream;
  /** Why its MCP server can't be used, if it can't. */
  error?: string;
  /** Resolves once the first tool listing finished (or failed). */
  ready: Promise<void>;
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function hashOf(tools: StoredTool[], instructions: string | undefined): string {
  const sorted = [...tools].sort((a, b) => a.name.localeCompare(b.name));
  return createHash("sha256")
    .update(JSON.stringify({ tools: sorted, instructions }))
    .digest("hex");
}

function sameEndpoint(a: McpEndpoint | undefined, b: McpEndpoint | undefined): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export class ConnectorRegistry {
  private readonly stored = new Map<string, StoredConnector>();
  private readonly live = new Map<string, Live>();
  private readonly listeners = new Set<() => void>();
  /** The library of every known app's components; built when asked, dropped when one changes. */
  private apps: AppLibrary | undefined;

  constructor(
    private readonly store: SessionStore,
    private readonly connect: ConnectUpstream,
    private readonly log: Logger,
  ) {
    for (const connector of store.loadConnectors()) this.stored.set(connector.app, connector);
  }

  /**
   * Registers, updates or (open: false) unregisters a connector. Resolves once its tools are
   * listed, or listing failed (then the connector is open with an error).
   */
  async presence(owner: object, sent: AppPresence): Promise<void> {
    const { app } = sent;
    const existing = this.live.get(app);
    if (!sent.open) {
      if (existing?.owner === owner) await this.drop(app, existing);
      return;
    }
    // A component that can't be used is left out; the app and its other components register.
    let presence = sent;
    if (sent.components !== undefined) {
      const { usable, problems } = checkComponents(sent.components);
      for (const problem of problems) this.log.warn(`${app}: ${problem}`);
      presence = { ...sent, components: usable };
    }
    this.remember(presence);
    if (existing?.owner === owner && sameEndpoint(existing.presence.mcp, presence.mcp)) {
      existing.presence = presence; // title/context/launch update
      this.notify();
      return existing.ready;
    }
    if (existing) await this.drop(app, existing); // re-registered by another client or endpoint
    let markReady = (): void => {};
    const live: Live = { owner, presence, ready: new Promise((resolve) => (markReady = resolve)) };
    this.live.set(app, live);
    this.notify();
    if (presence.mcp) await this.attach(app, live, presence.mcp);
    markReady();
  }

  /**
   * Updates what a registered app shows (its presence context): memory only, no broadcast, as
   * it can change on every cursor move. False if `owner` hasn't registered `app`.
   */
  setContext(owner: object, app: string, context: Record<string, string>): boolean {
    const live = this.live.get(app);
    if (live?.owner !== owner) return false;
    live.presence = { ...live.presence, context };
    return true;
  }

  /** Unregisters everything a bus client registered (its connection closed). */
  async disconnect(owner: object): Promise<void> {
    const owned = [...this.live].filter(([, live]) => live.owner === owner);
    await Promise.all(owned.map(([app, live]) => this.drop(app, live)));
  }

  /** The presence an open connector registered with. */
  presenceOf(app: string): AppPresence | undefined {
    return this.live.get(app)?.presence;
  }

  /** The open app that hosts the agent when no other app has it (the overlay), if any. */
  home(): string | undefined {
    for (const [app, live] of this.live)
      if (live.presence.home && live.presence.kind === "app") return app;
    return undefined;
  }

  /** The view template for a tool's results (`app.tool`), kept after the app closes. */
  template(tool: string): unknown {
    const dot = tool.indexOf(".");
    if (dot <= 0) return undefined;
    return this.stored.get(tool.slice(0, dot))?.templates?.[tool.slice(dot + 1)];
  }

  /**
   * The neoui components of the apps we know, by app: those of closed apps too, so docs that use
   * them still draw. Each is written with its app (`photo-editor.BrightnessCard`), so apps never
   * compete for a name and the order does not matter.
   */
  componentsOf(): AppComponents {
    const out: AppComponents = {};
    const known = [...this.stored.values()].sort((a, b) => a.app.localeCompare(b.app));
    for (const connector of known) {
      if (connector.components && Object.keys(connector.components).length > 0) {
        out[connector.app] = connector.components;
      }
    }
    return out;
  }

  /** The apps that are connected now, the ones whose components an agent is told about. */
  openApps(): Set<string> {
    return new Set(this.live.keys());
  }

  /** The suite library with every known app's components (see `componentsOf`). */
  library(): AppLibrary {
    this.apps ??= appLibrary(this.componentsOf());
    return this.apps;
  }

  isOpen(app: string): boolean {
    return this.live.has(app);
  }

  connector(app: string): StoredConnector | undefined {
    return this.stored.get(app);
  }

  upstream(app: string): { upstream?: Upstream; error?: string } | undefined {
    const live = this.live.get(app);
    if (!live) return undefined;
    return { upstream: live.upstream, error: live.error };
  }

  /** Every connector we know of, open ones first. */
  all(): { connector: StoredConnector; open: boolean }[] {
    return [...this.stored.values()]
      .map((connector) => ({ connector, open: this.live.has(connector.app) }))
      .sort(
        (a, b) => Number(b.open) - Number(a.open) || a.connector.app.localeCompare(b.connector.app),
      );
  }

  list(): ConnectorInfo[] {
    return this.all().map(({ connector, open }) => {
      const info: ConnectorInfo = {
        app: connector.app,
        kind: connector.kind,
        connected: open,
        v: connector.v,
        tools: connector.tools.map((t) => ({
          name: `${connector.app}.${t.name}`,
          description: t.description,
        })),
      };
      if (connector.title !== undefined) info.title = connector.title;
      const live = this.live.get(connector.app);
      if (live?.error !== undefined) info.error = live.error;
      if (live?.presence.home) info.home = true;
      if (live?.presence.pid !== undefined) info.pid = live.presence.pid;
      if (live?.presence.context !== undefined) info.context = live.presence.context;
      return info;
    });
  }

  /** Open connectors, as announced to sessions. */
  snapshot(): ConnectorSnapshot[] {
    const out: ConnectorSnapshot[] = [];
    for (const { connector, open } of this.all()) {
      if (!open) continue;
      const { app, kind, v, tools, instructions } = connector;
      const snapshot: ConnectorSnapshot = { app, kind, v, tools: tools.map((t) => t.name) };
      const components = callNames(this.library())[app] ?? [];
      if (components.length > 0) snapshot.components = components;
      const described = this.live.get(app)?.presence.description;
      if (described !== undefined && described !== "") snapshot.description = described;
      else if (instructions !== undefined && instructions.trim() !== "") {
        snapshot.description = firstSentence(instructions);
      }
      out.push(snapshot);
    }
    return out;
  }

  /** Resolves true once the app is open and its tools are listed, false on timeout. */
  async waitForOpen(app: string, timeoutMs: number): Promise<boolean> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const live = this.live.get(app);
      if (live) {
        await Promise.race([live.ready, Bun.sleep(deadline - Date.now())]);
        return this.live.has(app);
      }
      await new Promise<void>((resolve) => {
        const unsubscribe = this.onChange(() => {
          unsubscribe();
          resolve();
        });
        setTimeout(resolve, Math.max(0, deadline - Date.now()));
      });
    }
    return this.live.has(app);
  }

  onChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Closes every MCP connection (daemon shutdown). */
  async close(): Promise<void> {
    await Promise.all([...this.live].map(([app, live]) => this.drop(app, live)));
  }

  private remember(presence: AppPresence): void {
    const record: StoredConnector = this.stored.get(presence.app) ?? {
      app: presence.app,
      kind: presence.kind,
      v: 0,
      schemaHash: "",
      tools: [],
    };
    record.kind = presence.kind;
    if (presence.title !== undefined) record.title = presence.title;
    if (presence.launch !== undefined) record.launch = presence.launch;
    if (presence.templates !== undefined) record.templates = presence.templates;
    if (presence.components !== undefined) {
      const same = JSON.stringify(record.components ?? {}) === JSON.stringify(presence.components);
      if (!same) {
        // A session that knows an earlier version is told which components there are now.
        if (record.v > 0) record.v += 1;
        this.apps = undefined;
      }
      if (Object.keys(presence.components).length === 0) delete record.components;
      else record.components = presence.components;
    }
    this.stored.set(presence.app, record);
    this.store.saveConnector(record);
  }

  private async attach(app: string, live: Live, endpoint: McpEndpoint): Promise<void> {
    try {
      const upstream = await this.connect(endpoint);
      if (this.live.get(app) !== live) {
        await upstream.close(); // unregistered while connecting
        return;
      }
      live.upstream = upstream;
      upstream.onToolsChanged(() => void this.refresh(app, live));
      await this.refresh(app, live);
    } catch (err) {
      live.error = `can't reach its MCP server: ${errorText(err)}`;
      this.log.warn(`${app}: ${live.error}`);
      this.notify();
    }
  }

  private async refresh(app: string, live: Live): Promise<void> {
    if (!live.upstream) return;
    try {
      const tools = await live.upstream.listTools();
      const record = this.stored.get(app);
      if (this.live.get(app) !== live || !record) return;
      const instructions = live.upstream.instructions();
      const hash = hashOf(tools, instructions);
      if (record.schemaHash !== hash) {
        record.v += 1;
        record.schemaHash = hash;
        record.tools = tools;
        if (instructions === undefined) delete record.instructions;
        else record.instructions = instructions;
        this.store.saveConnector(record);
        this.log.info(`${app}: ${tools.length} tools, v${record.v}`);
      }
      delete live.error;
    } catch (err) {
      live.error = `listing its tools failed: ${errorText(err)}`;
      this.log.warn(`${app}: ${live.error}`);
    }
    this.notify();
  }

  private async drop(app: string, live: Live): Promise<void> {
    if (this.live.get(app) === live) this.live.delete(app);
    this.notify();
    await live.upstream?.close();
  }

  private notify(): void {
    for (const listener of this.listeners) listener();
  }
}

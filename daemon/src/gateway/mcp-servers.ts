// MCP servers the daemon runs itself (#22): programs it starts over stdio (and remote ones
// over HTTP) from the user's configuration and plugins (harness/src/mcp-config.ts). Each is
// registered with the connector registry as a service named `mcp-<name>`, so its tools reach
// agents through tools_search and tools_call like any connector's. A server that dies is started
// again after a growing pause; they all stop with the daemon.
import type { McpServerSpec } from "@neoworks-dev/neo/mcp-config";
import type { McpEndpoint } from "@neoworks-dev/neo-protocol/types";
import type { Logger } from "../log.ts";
import type { ConnectorRegistry } from "./registry.ts";
import {
  type ConnectUpstream,
  connectStdio as defaultConnectStdio,
  type StdioConnection,
  type Upstream,
} from "./upstream.ts";

export interface McpServersOptions {
  /** Starts a stdio server; tests give their own. */
  connectStdio?: typeof defaultConnectStdio;
  /** The pause after the first failure, doubled after each next one (default 1 s). */
  backoffMs?: number;
  /** The longest pause (default 60 s). */
  maxBackoffMs?: number;
  /** A server that ran this long was healthy: its next death starts the pauses over (default 60 s). */
  stableMs?: number;
  /** After this many failures in a row it is left alone until the configuration changes (default 8). */
  maxFailures?: number;
}

type State = "starting" | "up" | "waiting" | "failed" | "stopped";

interface Entry {
  spec: McpServerSpec;
  /** Changes with every start, so a late result of an earlier one is dropped. */
  run: number;
  state: State;
  failures: number;
  startedAt: number;
  error?: string;
  timer?: ReturnType<typeof setTimeout>;
  connection?: StdioConnection;
}

/** What the daemon says about a server (the /mcp view, tests). */
export interface McpServerStatus {
  app: string;
  name: string;
  origin: string;
  state: State;
  error?: string;
}

const STDIO_URL = "stdio://";

export class McpServers {
  private readonly owner = {};
  private readonly entries = new Map<string, Entry>();
  /** Started processes the registry has not taken yet, by the url it connects to. */
  private readonly prepared = new Map<string, Upstream>();

  constructor(
    private readonly registry: ConnectorRegistry,
    private readonly log: Logger,
    private readonly options: McpServersOptions = {},
  ) {}

  /** The registry's connect function: our started processes, and `base` for everything else. */
  connect(base: ConnectUpstream): ConnectUpstream {
    return (endpoint: McpEndpoint) => {
      if (!endpoint.url.startsWith(STDIO_URL)) return base(endpoint);
      const upstream = this.prepared.get(endpoint.url);
      this.prepared.delete(endpoint.url);
      return upstream ? Promise.resolve(upstream) : Promise.reject(new Error("no such process"));
    };
  }

  /**
   * Makes these the servers that run: new ones start, ones that are gone or changed stop (and a
   * changed one starts again). Resolves once every start has been tried once.
   */
  async set(specs: McpServerSpec[]): Promise<void> {
    const wanted = new Map(specs.map((spec) => [spec.app, spec]));
    const stops: Promise<void>[] = [];
    for (const [app, entry] of this.entries) {
      const spec = wanted.get(app);
      if (spec && JSON.stringify(spec.config) === JSON.stringify(entry.spec.config)) {
        entry.spec = spec; // same server, maybe from another source now
        continue;
      }
      this.entries.delete(app);
      stops.push(this.stop(entry));
    }
    await Promise.all(stops);
    const starts: Promise<void>[] = [];
    for (const spec of specs) {
      if (this.entries.has(spec.app)) continue;
      const entry: Entry = { spec, run: 0, state: "starting", failures: 0, startedAt: 0 };
      this.entries.set(spec.app, entry);
      starts.push(this.start(entry));
    }
    await Promise.all(starts);
  }

  status(): McpServerStatus[] {
    return [...this.entries.values()].map(({ spec, state, error }) => {
      const out: McpServerStatus = { app: spec.app, name: spec.name, origin: spec.origin, state };
      if (error !== undefined) out.error = error;
      return out;
    });
  }

  /** Stops every server (the daemon is shutting down); none starts again. */
  async close(): Promise<void> {
    const entries = [...this.entries.values()];
    this.entries.clear();
    await Promise.all(entries.map((entry) => this.stop(entry)));
  }

  private async start(entry: Entry): Promise<void> {
    const { spec } = entry;
    const run = ++entry.run;
    entry.state = "starting";
    entry.startedAt = Date.now();
    const current = (): boolean => entry.run === run && this.entries.get(spec.app) === entry;
    const title = `${spec.name} (MCP)`;
    try {
      let endpoint: McpEndpoint;
      if (spec.config.type === "http") {
        endpoint = { url: spec.config.url };
        if (Object.keys(spec.config.headers).length > 0) endpoint.headers = spec.config.headers;
      } else {
        const { command, args, env, cwd } = spec.config;
        const spawn = this.options.connectStdio ?? defaultConnectStdio;
        const connection = await spawn(
          cwd === undefined ? { command, args, env } : { command, args, env, cwd },
        );
        if (!current()) {
          await connection.upstream.close();
          return;
        }
        entry.connection = connection;
        endpoint = { url: `${STDIO_URL}${spec.app}` };
        this.prepared.set(endpoint.url, connection.upstream);
        connection.onExit((reason) => {
          if (current()) void this.died(entry, reason);
        });
      }
      await this.registry.presence(this.owner, {
        app: spec.app,
        kind: "service",
        title,
        open: true,
        mcp: endpoint,
      });
      this.prepared.delete(endpoint.url);
      if (!current()) return;
      const state = this.registry.upstream(spec.app);
      if (state?.upstream === undefined || state.error !== undefined) {
        throw new Error(state?.error ?? "its tools aren't listed");
      }
      entry.state = "up";
      delete entry.error;
      this.log.info(`${spec.app}: running (${spec.origin})`);
    } catch (err) {
      if (!current()) return;
      await this.died(entry, err instanceof Error ? err.message : String(err));
    }
  }

  /** A server that failed to start or went away: out of the registry, and started again later. */
  private async died(entry: Entry, reason: string): Promise<void> {
    const { spec } = entry;
    entry.error = reason;
    const run = ++entry.run;
    const connection = entry.connection;
    delete entry.connection;
    await this.registry.presence(this.owner, {
      app: spec.app,
      kind: "service",
      open: false,
    });
    await connection?.upstream.close();
    if (entry.run !== run || this.entries.get(spec.app) !== entry) return;
    const {
      backoffMs = 1000,
      maxBackoffMs = 60_000,
      stableMs = 60_000,
      maxFailures = 8,
    } = this.options;
    if (entry.startedAt > 0 && Date.now() - entry.startedAt >= stableMs) entry.failures = 0;
    entry.failures += 1;
    if (entry.failures > maxFailures) {
      entry.state = "failed";
      this.log.warn(`${spec.app}: ${reason}; giving up after ${maxFailures} tries`);
      return;
    }
    const wait = Math.min(maxBackoffMs, backoffMs * 2 ** (entry.failures - 1));
    entry.state = "waiting";
    this.log.warn(`${spec.app}: ${reason}; starting again in ${Math.round(wait / 100) / 10} s`);
    entry.timer = setTimeout(() => {
      delete entry.timer;
      if (this.entries.get(spec.app) === entry) void this.start(entry);
    }, wait);
  }

  private async stop(entry: Entry): Promise<void> {
    entry.state = "stopped";
    entry.run += 1;
    clearTimeout(entry.timer);
    await this.registry.presence(this.owner, {
      app: entry.spec.app,
      kind: "service",
      open: false,
    });
    await entry.connection?.upstream.close();
    delete entry.connection;
  }
}

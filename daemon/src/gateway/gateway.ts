// The core connector tools: tools_search, tools_call, apps_launch, apps_focus. Every outcome is a
// normal tool result (isError for failures), so the model can read it and correct itself.
import { spawn } from "node:child_process";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type {
  AppArgs,
  CoreToolName,
  ToolsCallArgs,
  ToolsSearchArgs,
} from "@suite/protocol/core-tools";
import Ajv2020, { type ValidateFunction } from "ajv/dist/2020.js";
import type { Logger } from "../log.ts";
import type { StoredConnector, StoredTool } from "../store.ts";
import type { ConnectorRegistry } from "./registry.ts";
import { type SearchHit, searchTools } from "./search.ts";

const LAUNCH_TIMEOUT_MS = 20_000;

export interface GatewayOptions {
  log: Logger;
  launchTimeoutMs?: number;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function displayName(connector: StoredConnector): string {
  return connector.title ?? connector.app;
}

function closedMessage(connector: StoredConnector): string {
  const name = displayName(connector);
  if (connector.launch) {
    return `${name} isn't open. Ask the user to open it, or call apps_launch('${connector.app}').`;
  }
  return `${name} isn't open. Ask the user to open it.`;
}

function formatHits(hits: SearchHit[]): string {
  const parts: string[] = [];
  let lastApp = "";
  for (const { connector, tool, open } of hits) {
    if (connector.app !== lastApp) {
      lastApp = connector.app;
      const state = open ? "open" : "closed";
      parts.push(`# ${connector.app} (${connector.kind}, v${connector.v}, ${state})`);
      if (connector.instructions) parts.push(connector.instructions.trim());
    }
    const lines = [`## ${connector.app}.${tool.name}`, tool.description];
    lines.push(`Input schema: ${JSON.stringify(tool.inputSchema)}`);
    if (tool.outputSchema) lines.push(`Output schema: ${JSON.stringify(tool.outputSchema)}`);
    parts.push(lines.join("\n"));
  }
  return parts.join("\n\n");
}

export class Gateway {
  private readonly ajv = new Ajv2020({ strict: false, allErrors: true });
  private readonly validators = new Map<string, ValidateFunction>();

  constructor(
    private readonly registry: ConnectorRegistry,
    private readonly options: GatewayOptions,
  ) {}

  async call(name: CoreToolName, args: unknown, signal?: AbortSignal): Promise<CallToolResult> {
    if (!isRecord(args)) return text(`${name}: arguments must be an object`, true);
    switch (name) {
      case "tools_search":
        if (typeof args.query !== "string") return text("tools_search: query is required", true);
        return this.search(args as unknown as ToolsSearchArgs);
      case "tools_call":
        if (typeof args.name !== "string") return text("tools_call: name is required", true);
        if (args.args !== undefined && !isRecord(args.args)) {
          return text("tools_call: args must be an object", true);
        }
        return this.callTool(args as unknown as ToolsCallArgs, signal);
      case "apps_launch":
        if (typeof args.app !== "string") return text("apps_launch: app is required", true);
        return this.launch(args as unknown as AppArgs);
      case "apps_focus":
        // Moving the agent between apps arrives with handoff (Phase 3).
        return text("Moving into apps isn't available yet; keep working from here.", true);
    }
  }

  private search({ query, app }: ToolsSearchArgs): CallToolResult {
    const all = this.registry.all();
    if (app !== undefined && !all.some((c) => c.connector.app === app)) {
      return text(`No connector named "${app}". Known: ${this.knownApps()}.`, true);
    }
    const hits = searchTools(all, query, app);
    if (hits.length === 0) {
      return text(`No tools match "${query}". Known connectors: ${this.knownApps()}.`);
    }
    const result = text(formatHits(hits));
    result.structuredContent = {
      tools: hits.map(({ connector, tool, open }) => ({
        name: `${connector.app}.${tool.name}`,
        app: connector.app,
        open,
        description: tool.description,
        inputSchema: tool.inputSchema,
        outputSchema: tool.outputSchema,
      })),
    };
    return result;
  }

  private async callTool(
    { name, args = {} }: ToolsCallArgs,
    signal?: AbortSignal,
  ): Promise<CallToolResult> {
    const dot = name.indexOf(".");
    const app = dot > 0 ? name.slice(0, dot) : name;
    const toolName = name.slice(dot + 1);
    const connector = this.registry.connector(app);
    if (dot <= 0 || !connector) {
      return text(
        `Unknown tool "${name}". Tool names look like app.tool; find them with tools_search. Known connectors: ${this.knownApps()}.`,
        true,
      );
    }
    const tool = connector.tools.find((t) => t.name === toolName);
    if (!tool) {
      return text(
        `${displayName(connector)} has no tool "${toolName}". Use tools_search with app "${app}" to list its tools.`,
        true,
      );
    }
    const live = this.registry.upstream(app);
    if (!live) return text(closedMessage(connector), true);
    if (!live.upstream) {
      return text(
        `${displayName(connector)} is open, but ${live.error ?? "its tools aren't ready"}.`,
        true,
      );
    }
    const problem = this.validate(connector, tool, args);
    if (problem !== undefined) return text(problem, true);
    try {
      return await live.upstream.call(toolName, args, signal);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return text(`${name} failed: ${message}`, true);
    }
  }

  private validate(
    connector: StoredConnector,
    tool: StoredTool,
    args: Record<string, unknown>,
  ): string | undefined {
    const key = `${connector.app}\0${connector.v}\0${tool.name}`;
    let validator = this.validators.get(key);
    if (!validator) {
      try {
        validator = this.ajv.compile(tool.inputSchema);
      } catch {
        return undefined; // an uncompilable schema is the app's problem; let it validate itself
      }
      this.validators.set(key, validator);
    }
    if (validator(args)) return undefined;
    const errors = (validator.errors ?? [])
      .map((e) => `${e.instancePath || "(root)"} ${e.message ?? "is invalid"}`)
      .join("; ");
    return `Invalid arguments for ${connector.app}.${tool.name}: ${errors}. Input schema: ${JSON.stringify(tool.inputSchema)}`;
  }

  private async launch({ app }: AppArgs): Promise<CallToolResult> {
    const connector = this.registry.connector(app);
    if (!connector) return text(`No connector named "${app}". Known: ${this.knownApps()}.`, true);
    if (this.registry.isOpen(app)) return text(`${displayName(connector)} is already open.`);
    const command = connector.launch?.command;
    if (!command || command.length === 0) {
      return text(
        `I don't know how to open ${displayName(connector)}. Ask the user to open it.`,
        true,
      );
    }
    try {
      const [program, ...rest] = command;
      const child = spawn(program ?? "", rest, {
        cwd: connector.launch?.cwd,
        detached: true,
        stdio: "ignore",
      });
      child.on("error", (err) => this.options.log.warn(`launching ${app}: ${err.message}`));
      child.unref();
    } catch (err) {
      return text(`Launching ${displayName(connector)} failed: ${String(err)}`, true);
    }
    const timeout = this.options.launchTimeoutMs ?? LAUNCH_TIMEOUT_MS;
    if (!(await this.registry.waitForOpen(app, timeout))) {
      return text(
        `${displayName(connector)} was started but didn't connect within ${timeout / 1000}s.`,
        true,
      );
    }
    const tools = this.registry.connector(app)?.tools.map((t) => t.name) ?? [];
    return text(`${displayName(connector)} is open. Tools: ${tools.join(", ")}.`);
  }

  private knownApps(): string {
    const all = this.registry.all();
    if (all.length === 0) return "none";
    return all
      .map(({ connector, open }) => `${connector.app} (${open ? "open" : "closed"})`)
      .join(", ");
  }
}

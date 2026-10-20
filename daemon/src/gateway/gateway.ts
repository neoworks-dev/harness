// The core connector tools: tools_search, tools_call, apps_launch, apps_focus, delegate, models,
// send_message, wait, ui_library. Every outcome is a normal tool result (isError for failures), so the model
// can read it and correct itself.
import { spawn } from "node:child_process";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type {
  AppArgs,
  CoreToolName,
  DelegateArgs,
  ModelsArgs,
  SendMessageArgs,
  ToolsCallArgs,
  ToolsSearchArgs,
  WaitArgs,
} from "@suite/protocol/core-tools";
import { uiLibraryText } from "@suite/openui/skill";
import type { ToolResult } from "@suite/protocol/types";
import Ajv2020, { type ValidateFunction } from "ajv/dist/2020.js";
import type { Logger } from "../log.ts";
import type { MessageTools } from "../messages.ts";
import type { StoredConnector, StoredTool } from "../store.ts";
import type { ConnectorRegistry } from "./registry.ts";
import { type SearchHit, searchTools } from "./search.ts";

const LAUNCH_TIMEOUT_MS = 20_000;

export interface GatewayOptions {
  log: Logger;
  /** Moves the agent for apps_focus; returns an error message or undefined. */
  focus?: (app: string, session: string | undefined) => string | undefined;
  launchTimeoutMs?: number;
  /** Runs a subagent for delegate; its final reply is the result. */
  delegate?: (session: string, args: DelegateArgs, signal?: AbortSignal) => Promise<CallToolResult>;
  /** What the subagent harnesses take for delegate's model and effort. */
  models?: (session: string, args: ModelsArgs) => Promise<CallToolResult>;
  /** send_message, wait, and the messages an ACP agent gets with its tool results. */
  messages?: MessageTools;
  /** Stores a result a guest gets as text with [result:ref], so views can draw it. */
  recordResult?: (session: string, result: ToolResult) => void;
  /** The file behind an asset:// handle, named in results for guests. */
  resolveAsset?: (handle: string) => string | undefined;
}

const ASSET_HANDLES = /asset:\/\/[a-z]+-[0-9a-f]+/g;

/**
 * Who is calling: the session (x-suite-session); `guest` (x-suite-guest) names a subagent, which
 * may not delegate; `text` (x-suite-text) is an ACP client (guests and the session's own ACP
 * harness) that gets text only.
 */
export interface Caller {
  session?: string;
  guest?: string;
  text?: boolean;
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

/**
 * tools_call's args: an object, or a JSON string of one (models sometimes send that). Arguments
 * under another key are an error that says where they go. A string is an error message.
 */
function toolsCallArgs(args: Record<string, unknown>): Record<string, unknown> | string {
  let value = args.args;
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return "tools_call: args must be an object (got a string that isn't JSON)";
    }
  }
  if (value === undefined) {
    const stray = Object.keys(args).filter((key) => key !== "name");
    if (stray.length > 0) {
      return `tools_call takes {name, args}: put the tool's arguments in args (an object), not in ${stray.join(", ")}.`;
    }
    return {};
  }
  if (!isRecord(value)) return "tools_call: args must be an object";
  return value;
}

/**
 * ACP clients get text only: some (Claude Code) show structuredContent instead of the text when
 * both are there, which would hide the [result:ref] and the formatted search results.
 */
function textOnly(result: CallToolResult): CallToolResult {
  const { structuredContent: _, ...rest } = result;
  return rest;
}

export class Gateway {
  private readonly ajv = new Ajv2020({ strict: false, allErrors: true });
  private readonly validators = new Map<string, ValidateFunction>();

  constructor(
    private readonly registry: ConnectorRegistry,
    private readonly options: GatewayOptions,
  ) {}

  /**
   * `caller` says whose agent calls: a session id, or the session and what kind of client. An
   * ACP agent can't be told anything mid-turn, so what other agents wrote to it comes with the
   * result of its next suite tool.
   */
  async call(
    name: CoreToolName,
    args: unknown,
    signal?: AbortSignal,
    caller: Caller | string = {},
  ): Promise<CallToolResult> {
    const result = await this.dispatch(name, args, signal, caller);
    if (typeof caller === "string") return result;
    const { session, guest, text: wantsText } = caller;
    if (wantsText !== true || session === undefined) return result;
    const waiting = this.options.messages?.take(session, guest) ?? [];
    if (waiting.length === 0) return result;
    const note: CallToolResult["content"][number] = { type: "text", text: waiting.join("\n\n") };
    return { ...result, content: [...result.content, note] };
  }

  private async dispatch(
    name: CoreToolName,
    args: unknown,
    signal: AbortSignal | undefined,
    caller: Caller | string,
  ): Promise<CallToolResult> {
    const {
      session,
      guest,
      text: wantsText,
    } = typeof caller === "string" ? { session: caller } : caller;
    if (!isRecord(args)) return text(`${name}: arguments must be an object`, true);
    switch (name) {
      case "tools_search": {
        if (typeof args.query !== "string") return text("tools_search: query is required", true);
        const found = this.search(args as unknown as ToolsSearchArgs);
        return wantsText === true ? textOnly(found) : found;
      }
      case "tools_call": {
        if (typeof args.name !== "string") return text("tools_call: name is required", true);
        const toolArgs = toolsCallArgs(args);
        if (typeof toolArgs === "string") return text(toolArgs, true);
        const call = { name: args.name, args: toolArgs };
        if (wantsText === true && session !== undefined) {
          return textOnly(this.withRef(session, args.name, await this.callTool(call, signal)));
        }
        return this.callTool(call, signal);
      }
      case "apps_launch":
        if (typeof args.app !== "string") return text("apps_launch: app is required", true);
        return this.launch(args as unknown as AppArgs);
      case "apps_focus":
        if (typeof args.app !== "string") return text("apps_focus: app is required", true);
        return this.moveInto(args.app, session);
      case "delegate":
        if (typeof args.agent !== "string") return text("delegate: agent is required", true);
        if (typeof args.task !== "string" || args.task.trim() === "") {
          return text("delegate: task is required", true);
        }
        for (const key of ["model", "effort", "cwd", "worktree"]) {
          if (args[key] !== undefined && typeof args[key] !== "string") {
            return text(`delegate: ${key} must be a string`, true);
          }
        }
        if (args.background !== undefined && typeof args.background !== "boolean") {
          return text("delegate: background must be a boolean", true);
        }
        if (guest !== undefined) return text("Subagents can't delegate.", true);
        if (!this.options.delegate || session === undefined) {
          return text("No other agents are available here.", true);
        }
        return this.options.delegate(session, args as unknown as DelegateArgs, signal);
      case "models":
        for (const key of ["agent", "query"]) {
          if (args[key] !== undefined && typeof args[key] !== "string") {
            return text(`models: ${key} must be a string`, true);
          }
        }
        if (guest !== undefined) return text("Subagents can't delegate.", true);
        if (!this.options.models || session === undefined) {
          return text("No other agents are available here.", true);
        }
        return this.options.models(session, args as ModelsArgs);
      case "send_message":
        if (typeof args.to !== "string" || args.to.trim() === "") {
          return text("send_message: to is required", true);
        }
        if (typeof args.text !== "string" || args.text.trim() === "") {
          return text("send_message: text is required", true);
        }
        if (!this.options.messages || session === undefined) {
          return text("No other agents are available here.", true);
        }
        return this.options.messages.send(session, guest, args as unknown as SendMessageArgs);
      case "ui_library":
        return text(uiLibraryText(this.registry.library(), this.registry.openApps()));
      case "wait": {
        const names: unknown = args.for;
        if (
          names !== undefined &&
          !(Array.isArray(names) && names.every((n) => typeof n === "string"))
        ) {
          return text("wait: for must be a list of subagent ids", true);
        }
        const seconds: unknown = args.timeoutSeconds;
        if (seconds !== undefined && (typeof seconds !== "number" || !Number.isFinite(seconds))) {
          return text("wait: timeoutSeconds must be a number", true);
        }
        if (!this.options.messages || session === undefined) {
          return text("No other agents are available here.", true);
        }
        return this.options.messages.wait(session, guest, args as WaitArgs, signal);
      }
    }
  }

  /**
   * ACP clients reduce results to text, so a structured result is stored (tool_result) and the
   * text carries its ref; views match it and draw the result with the tool's template. Their own
   * file tools don't know asset:// handles, so the result also names the file behind each.
   */
  private withRef(session: string, tool: string, result: CallToolResult): CallToolResult {
    if (result.isError) return result;
    const extra: string[] = [];
    const handles = new Set(JSON.stringify(result).match(ASSET_HANDLES) ?? []);
    for (const handle of handles) {
      const path = this.options.resolveAsset?.(handle);
      if (path !== undefined) extra.push(`${handle} is the file ${path}`);
    }
    if (result.structuredContent !== undefined && this.options.recordResult) {
      const ref = crypto.randomUUID().slice(0, 8);
      this.options.recordResult(session, {
        type: "tool_result",
        ref,
        tool,
        rawOutput: result.structuredContent,
      });
      extra.push(`[result:${ref}]`);
    }
    if (extra.length === 0) return result;
    return { ...result, content: [...result.content, { type: "text", text: extra.join("\n") }] };
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

  private moveInto(app: string, session: string | undefined): CallToolResult {
    const connector = this.registry.connector(app);
    if (!connector) return text(`No connector named "${app}". Known: ${this.knownApps()}.`, true);
    if (!this.registry.isOpen(app)) return text(closedMessage(connector), true);
    if (!this.options.focus) return text("Moving between apps isn't available here.", true);
    const problem = this.options.focus(app, session);
    if (problem !== undefined) return text(problem, true);
    return text(
      `You're now in ${displayName(connector)}: its pane shows this conversation and the user continues there.`,
    );
  }

  private knownApps(): string {
    const all = this.registry.all();
    if (all.length === 0) return "none";
    return all
      .map(({ connector, open }) => `${connector.app} (${open ? "open" : "closed"})`)
      .join(", ");
  }
}

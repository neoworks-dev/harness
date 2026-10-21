// neo as a subagent (delegate {agent: "neo"}): a second neo conversation next to the session's
// agent, made with the same harness, so a session that runs on Claude Code can hand work to neo
// and the other way round. Its updates are labelled "neo" like a guest's and never reach neo's
// own history (no harness_message), and its gateway client may not delegate.
import type {
  Agent,
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  McpServer,
  NewSessionParams,
  PromptParams,
  RequestPermission,
} from "@neoworks-dev/neo/agent";
import type { ConnectorSnapshot } from "../gateway/registry.ts";
import type { Logger } from "../log.ts";
import type { HarnessSettings } from "../settings.ts";
import { type FocusTarget, ToldState } from "../told.ts";
import type { GuestRunResult, RunOptions } from "./run.ts";

export const NEO = "neo";

export interface SubagentOptions {
  log: Logger;
  cwd: (session: string) => string;
  /** The gateway for the subagent (marked as a guest: no delegate). */
  mcpServers: (session: string) => McpServer[];
  resolveAsset?: (handle: string) => string | undefined;
  /** The hook its bash, write and edit ask through (see permissions.ts). */
  requestPermission?: (session: string) => RequestPermission;
  /** What the user chose for neo in this session (model, effort). */
  settings: (session: string) => HarnessSettings;
  connectors: () => ConnectorSnapshot[];
  focus: (session: string) => FocusTarget | undefined;
}

interface Subagent {
  session: Promise<AgentSession>;
  told: ToldState;
  /** One prompt at a time. */
  queue: Promise<unknown>;
}

/** Labels an update as neo's own subagent work; undefined for what must stay out of the log. */
function labelled(update: AgentUpdate): AgentUpdate | undefined {
  switch (update.type) {
    case "harness_message":
      return undefined;
    case "agent_message_chunk":
    case "agent_thought_chunk":
      return { ...update, messageId: `${NEO}-${update.messageId}`, agent: NEO };
    case "tool_call":
      return { ...update, toolCallId: `${NEO}:${update.toolCallId}`, agent: NEO };
    case "tool_call_update":
      return { ...update, toolCallId: `${NEO}:${update.toolCallId}` };
    case "plan":
    case "usage":
      return { ...update, agent: NEO };
    case "notice":
      return update;
  }
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export class NeoSubagents {
  private readonly agents = new Map<string, Subagent>();

  constructor(
    private readonly neo: Agent,
    private readonly options: SubagentOptions,
  ) {}

  /** The session's neo subagent (one per agent definition too), made on first use. */
  private subagent(session: string, definition?: string): Subagent {
    const key = definition === undefined ? session : `${session}\0${definition}`;
    const existing = this.agents.get(key);
    if (existing) return existing;
    const created: Subagent = {
      session: this.create(session, definition),
      told: new ToldState(),
      queue: Promise.resolve(),
    };
    created.session.catch(() => this.agents.delete(key));
    this.agents.set(key, created);
    return created;
  }

  /** neo's models and thinking levels (in-process: nothing is started for the answer). */
  async configOptions(session: string): Promise<ConfigOption[] | undefined> {
    try {
      return (await this.subagent(session).session).configOptions?.();
    } catch {
      return undefined;
    }
  }

  /** Runs one task on the session's neo subagent (made on first use; follow-ups continue it). */
  async run(session: string, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const current = this.subagent(session, opts.definition);
    let agentSession: AgentSession;
    try {
      agentSession = await current.session;
    } catch (err) {
      return { stopReason: "error", error: `Starting neo failed: ${errorText(err)}`, text: "" };
    }
    const turn = current.queue.then(() => this.prompt(session, current, agentSession, task, opts));
    current.queue = turn.catch(() => {});
    return turn;
  }

  private create(session: string, definition?: string): Promise<AgentSession> {
    const { options } = this;
    const params: NewSessionParams = {
      sessionId: definition === undefined ? `${session}-neo` : `${session}-neo-${definition}`,
      cwd: options.cwd(session),
      mcpServers: options.mcpServers(session),
      subagent: { ...(definition !== undefined && { definition }) },
    };
    if (options.resolveAsset) params.resolveAsset = options.resolveAsset;
    const settings = options.settings(session);
    if (Object.keys(settings).length > 0) params.settings = settings;
    const hook = options.requestPermission?.(session);
    if (hook) params.requestPermission = this.prefixed(hook);
    return this.neo.newSession(params);
  }

  /** Its tool calls are logged as neo:<id>, so the question has to name them the same way. */
  private prefixed(hook: RequestPermission): RequestPermission {
    return (request, signal) => {
      const { toolCallId } = request;
      if (toolCallId === undefined) return hook(request, signal);
      return hook({ ...request, toolCallId: `${NEO}:${toolCallId}` }, signal);
    };
  }

  private async prompt(
    session: string,
    agent: Subagent,
    agentSession: AgentSession,
    task: string,
    opts: RunOptions,
  ): Promise<GuestRunResult> {
    const messages = new Map<string, string>();
    const text = (): string => [...messages.values()].at(-1)?.trim() ?? "";
    if (opts.signal?.aborted) return { stopReason: "cancelled", text: "" };
    const restore = await this.apply(agentSession, opts);
    if (typeof restore === "string") return { stopReason: "error", error: restore, text: "" };
    const events = agent.told.changes(this.options.connectors(), this.options.focus(session));
    for (const event of events) agent.told.apply(event);
    const onAbort = (): void => agentSession.cancel();
    opts.signal?.addEventListener("abort", onAbort);
    try {
      const params: PromptParams = {
        text: task,
        app: "delegate",
        pendingEvents: events,
        onUpdate: (update) => {
          const out = labelled(update);
          if (!out) return;
          if (out.type === "agent_message_chunk") {
            messages.set(out.messageId, (messages.get(out.messageId) ?? "") + out.text);
          }
          opts.emit(out);
        },
      };
      if (opts.images) params.images = opts.images;
      if (opts.takeMessages) params.takeMessages = opts.takeMessages;
      const result = await agentSession.prompt(params);
      return { ...result, text: text() };
    } finally {
      opts.signal?.removeEventListener("abort", onAbort);
      await restore();
    }
  }

  /**
   * Sets the task's model and effort; returns how to put them back, or the harness's complaint
   * (an unknown model).
   */
  private async apply(
    session: AgentSession,
    opts: RunOptions,
  ): Promise<(() => Promise<void>) | string> {
    const before: [ConfigCategory, string][] = [];
    const back = async (): Promise<void> => {
      for (const [category, value] of before) {
        try {
          await session.setConfigOption?.(category, value);
        } catch (err) {
          this.options.log.warn(
            `neo: putting ${category} back to ${value} failed: ${errorText(err)}`,
          );
        }
      }
    };
    for (const [name, value] of Object.entries(opts.settings ?? {})) {
      const category = name as ConfigCategory;
      const current = session.configOptions?.().find((o) => o.category === category)?.current;
      try {
        await session.setConfigOption?.(category, value);
      } catch (err) {
        await back();
        return errorText(err);
      }
      if (current !== undefined && current !== value) before.push([category, current]);
    }
    return back;
  }
}

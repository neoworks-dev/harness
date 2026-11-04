// The ACP bridge (design §9): starts agents (Claude Code, Codex) through their ACP adapters and
// streams what they do into neo session. Two roles: guests, one per neo session and
// name, that the session's agent reaches with delegate and the user with "/claude task"; and the
// session's own agent when the user runs it on that harness (/harness claude), which has no
// label in the log and gets the session's context events. neo subagents (subagent.ts) are
// delegated to the same way.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join } from "node:path";
import type {
  Agent,
  AgentInfo,
  AgentSession,
  AgentUpdate,
  ConfigCategory,
  ConfigOption,
  ContextEvent,
  GenerateTextParams,
  McpServer,
  PromptResult,
  RequestPermission,
} from "@neoworks-dev/neo/agent";
import { formatPrompt } from "@neoworks-dev/neo/context-events";
import { isCoreToolName } from "@neoworks-dev/neo-protocol/core-tools";
import type {
  PermissionOption,
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
  SessionEvent,
  SlashCommand,
  StopReason,
  ToolCall,
  ToolCallContent,
  ToolCallStatus,
  ToolCallUpdate,
  ToolKind,
  Usage,
} from "@neoworks-dev/neo-protocol/types";
import type { Asker, PermissionRequest } from "../asker.ts";
import { BadRequestError } from "../focus.ts";
import type { ConnectorSnapshot } from "../gateway/registry.ts";
import type { Logger } from "../log.ts";
import { NotFoundError } from "../sessions.ts";
import { type Guard, RUN_TITLE, type Touches } from "../permissions.ts";
import type { Approvals, HarnessSettings } from "../settings.ts";
import { type FocusTarget, type ShowDontTellSetting, ToldState } from "../told.ts";
import { AcpConnection, AcpError } from "./acp.ts";
import { CLAUDE_AGENT_TOOLS, withCodexSubagents } from "./config.ts";
import type { GuestAccounts, Route, RouteKind } from "./accounts.ts";
import { GUEST_PROMPT, MAIN_PROMPT } from "./prompt.ts";
import type { GuestRunResult, RunOptions } from "./run.ts";
import { NEO, NeoSubagents, type SubagentOptions } from "./subagent.ts";

export interface GuestConfig {
  /** Shown to the main agent, e.g. "Claude Code". */
  title: string;
  /** The ACP adapter. */
  command: string[];
  env?: Record<string, string>;
  /**
   * Which of neo's API keys the guest can run on (see accounts.ts): `/model <name>
   * provider/model` then starts it on that key. Unset: only its own login.
   */
  route?: RouteKind;
  /** ask: permission requests show as buttons in the chat; allow: the first allow option wins. */
  permissions: "ask" | "allow";
  /**
   * It has subagents of its own that neo turns on and off: asks, their starts come as permission
   * requests (Claude Code); silent, they start without asking (Codex).
   */
  subagents?: "asks" | "silent";
}

/** Which client of the gateway an agent is (see Caller in gateway.ts). */
export interface GatewayRole {
  name: string;
  /** A subagent: it may not delegate. */
  guest?: boolean;
  /** An ACP client: it gets text-only results. */
  text?: boolean;
}

export interface GuestManagerOptions {
  configs: Record<string, GuestConfig>;
  log: Logger;
  /** Appends to a session's log. */
  append: (session: string, event: SessionEvent) => void;
  cwd: (session: string) => string;
  /** The gateway, with headers that say who calls. */
  mcpServers: (session: string, role: GatewayRole) => McpServer[];
  connectors: () => ConnectorSnapshot[];
  focus: (session: string) => FocusTarget | undefined;
  /** The user's "Show, don't tell." setting, told to each adapter when it changed. */
  showDontTell?: () => ShowDontTellSetting;
  /** What the user chose for this harness in this session (model, effort, mode). */
  settings: (session: string, harness: string) => HarnessSettings;
  /** Asks the user (permission requests). */
  asker: Asker;
  /**
   * The session's approvals mode (/approvals): auto allows everything, edits and review allow
   * edits (review: the card comes after the edit, see review.ts). Reads are allowed in every
   * mode.
   */
  approvals?: (session: string) => Approvals;
  /** Reads and commands that touch sensitive files ask in every mode (see permissions.ts). */
  guard?: Guard;
  /**
   * A summary of the turns other harnesses ran (see handoff.ts), for an ACP harness that lost
   * its own conversation: `all` asks for every turn so far.
   */
  handoff?: (session: string, harness: string, all: boolean) => string | undefined;
  /** neo, for delegate {agent: "neo"} (its subagent sessions are made with it). */
  neo?: Agent;
  resolveAsset?: (handle: string) => string | undefined;
  /** Stores an image an adapter sent (base64) and returns its asset handle. */
  storeImage?: StoreImage;
  /** The hook a neo subagent's own bash, write and edit ask through. */
  requestPermission?: (session: string, agent: string) => RequestPermission;
  /** Neo's API keys, for guests with a `route`. */
  accounts?: GuestAccounts;
  /**
   * Whether a session's agent may start subagents of its own (Claude Code's Agent tool, Codex's
   * multi-agent tools), read when its adapter starts. Never for a subagent. Default: no.
   */
  nativeSubagents?: (session: string) => boolean;
  /**
   * Makes the session a native subagent of `session`'s agent works as: a child of it on this
   * harness, so it is listed and opens like a delegation's; the one it worked in before when
   * there is one (`handle`, its id in the harness, after a restart). Without it the subagent's
   * events go into `session`, labelled.
   */
  childSession?: (session: string, harness: string, handle: string) => string;
  /**
   * The session's agent went on by itself (see OwnTurn): the session should run a turn for it,
   * which ownTurnSession shows. `title` is the harness's. Without it that work isn't shown.
   */
  ownTurn?: (session: string, title: string) => void;
  /** Stops the session's running turn alone (see SessionManager.stopTurn). */
  stopTurn?: (session: string) => void;
  /** Whether starting a subagent asks in every mode (the user's /settings ask-for-agents). */
  alwaysAskForAgents?: () => boolean;
}

/** A conversation the adapter has stored (session/list), for /resume. */
export interface StoredConversation {
  id: string;
  title?: string;
  updatedAt?: string;
}

/** Where the updates of one ACP session go: a turn's, or a native subagent's of that turn. */
interface Stream {
  emit: (update: AgentUpdate) => void;
  /** The label its events carry; the session's own agent's have none. */
  agent?: string;
  /** Before its message and tool call ids, so they stay unique in the log. */
  prefix: string;
  /** ACP may not send message ids: a new one starts whenever something else came in between. */
  messageId?: string;
  messageKind?: "agent_message_chunk" | "agent_thought_chunk";
  /** Text of the agent's messages, by id, in order. */
  messages: Map<string, string>;
  /** The adapter's own title of each tool call, to describe it again once its input arrives. */
  titles: Map<string, string>;
}

interface Run extends Stream {
  /** Aborted when the run ends: drops its open permission requests. */
  done: AbortController;
}

/**
 * A subagent of the harness's own (Claude Code's Agent tool, Codex's spawn_agent), which the
 * adapter reports as an ACP session of its own (the `subagents` client capability). It is logged
 * like a delegation into a directory: subagent_started and subagent_ended in the session, and
 * its work in a session of its own (`child`), each run a turn there that its task starts. So the
 * lists show it and it opens like a delegated agent's session.
 */
interface NativeChild {
  /** Its ACP session, which is its id in the harness too (Claude Code's agentId). */
  handle: string;
  /** The session it works in (see GuestManagerOptions.childSession). */
  session?: string;
  /** Takes back the copies of its questions in the session that started it. */
  unmirror?: () => void;
  /** Its subagent id in subagent_started; a new one each time the adapter runs it again. */
  id: string;
  /** What lists call it: its name and role (Explore), when the adapter says. */
  name?: string;
  /** Claude Code's short description of the task, until the prompt arrives. */
  description?: string;
  /** Between its subagent_started and subagent_ended. */
  running: boolean;
  /** subagent_started is logged once something of this run arrived. */
  started: boolean;
  /** It may outlive the turn (Claude Code's run_in_background, every Codex agent). */
  background: boolean;
  /** session/cancel on its ACP session stops it. */
  cancel: boolean;
  stream: Stream;
}

/**
 * Work the session's own agent does without a prompt: Claude Code's follow-up once a command or
 * subagent it ran in the background ended (its "autonomous cycle"), which ACP v1 reports only as
 * updates outside any prompt, ending with its result's usage. The daemon runs a turn of the
 * session for it (see GuestManagerOptions.ownTurn), so it is shown and nothing else runs meanwhile.
 */
interface OwnTurn {
  /** What arrived before that turn started. */
  held: Record<string, unknown>[];
  /** Set once it is over. */
  stopReason?: StopReason;
  /** The turn that shows it, once that started. */
  run?: Run;
  /** Ends that turn. */
  end?: () => void;
  /** Ends it when nothing came for a while, so a result without usage can't hang the session. */
  quiet?: ReturnType<typeof setTimeout>;
}

/** What starts an own turn of the agent: its output, not a late update of an earlier turn. */
const OWN_TURN_STARTS = new Set([
  "agent_message_chunk",
  "agent_thought_chunk",
  "tool_call",
  "plan",
]);
/** How long an own turn of the agent may go without an update before it counts as over. */
const OWN_TURN_QUIET_MS = 10 * 60 * 1000;

/** Where a connection's notifications go: its guest, once there is one. */
interface GuestTarget {
  guest?: Guest;
  /** Listed before the guest existed. */
  commands?: SlashCommand[];
}

/** An adapter's availableCommands as menu entries; one without a usable name is dropped. */
export function slashCommands(value: unknown): SlashCommand[] {
  const out: SlashCommand[] = [];
  if (!Array.isArray(value)) return out;
  for (const item of value) {
    if (!isRecord(item)) continue;
    const name = str(item.name)?.replace(/^\/+/, "");
    if (name === undefined || name === "" || /\s/.test(name)) continue;
    const command: SlashCommand = { name };
    const description = str(item.description)?.replace(/\s+/g, " ").trim();
    if (description) command.description = description;
    const hint = isRecord(item.input) ? str(item.input.hint) : undefined;
    if (hint) command.hint = hint;
    if (!out.some((other) => other.name === name)) out.push(command);
  }
  return out;
}

interface Guest {
  name: string;
  session: string;
  /** The session's own agent, not a guest: its updates carry no agent label. */
  main: boolean;
  /** A conversation the harness doesn't carry from before (new, or its stored one is gone). */
  fresh: boolean;
  conn: AcpConnection;
  acpSession: string;
  told: ToldState;
  /** One prompt at a time per guest. */
  queue: Promise<unknown>;
  run?: Run;
  /** The adapter's config options (session/new, set_config_option), by ACP config id. */
  config: Map<string, ConfigOption>;
  /** The API key it runs on, when the user picked one; else it uses its own login. */
  route?: Route;
  /** The slash commands the adapter last listed (available_commands_update). */
  commands?: SlashCommand[];
  /** The adapter takes `_session/steering`: a message for the running turn. */
  steering: boolean;
  /** The adapter takes `_session/background_tasks`: a running tool call's task goes on aside. */
  backgroundTasks: boolean;
  /** What a slash command's prompt couldn't carry, for the next prompt. */
  carried?: { events: ContextEvent[]; summary?: string };
  /** Its own subagents, by the ACP session they report in. */
  children: Map<string, NativeChild>;
  /** Whether each of its Agent calls runs in the background, by the adapter's tool call id. */
  backgroundCalls: Map<string, boolean>;
  /** What it does on its own right now, as the session's agent. */
  own?: OwnTurn;
}

const KINDS = new Set<string>([
  "read",
  "edit",
  "delete",
  "move",
  "search",
  "execute",
  "think",
  "fetch",
  "other",
]);
const STATUSES = new Set<string>(["pending", "in_progress", "completed", "failed"]);
const PLAN_STATUSES: PlanEntryStatus[] = ["pending", "in_progress", "completed"];
const PLAN_PRIORITIES: PlanEntryPriority[] = ["high", "medium", "low"];
/** ACP's extension method for a message to the turn that is running. */
const STEER_METHOD = "_session/steering";

/** A note the user added to a guest's permission answer: the question, the answer, the note. */
interface SaidNote {
  title: string;
  answer: string;
  note: string;
}
/** Sends a running tool call's task to the background (an adapter that advertises it). */
const BACKGROUND_METHOD = "_session/background_tasks";
const STOP_REASONS = new Set<string>(["end_turn", "max_tokens", "cancelled", "refusal"]);
const IMAGE_TYPES: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function count(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? Math.round(value)
    : undefined;
}

/** ACP plan entries; one without content is dropped, an unknown status counts as pending. */
function planEntries(value: unknown): PlanEntry[] {
  const entries: PlanEntry[] = [];
  if (!Array.isArray(value)) return entries;
  for (const item of value) {
    const content = isRecord(item) ? str(item.content) : undefined;
    if (!isRecord(item) || content === undefined) continue;
    const entry: PlanEntry = {
      content,
      status: PLAN_STATUSES.find((status) => status === item.status) ?? "pending",
    };
    const priority = PLAN_PRIORITIES.find((candidate) => candidate === item.priority);
    if (priority !== undefined) entry.priority = priority;
    entries.push(entry);
  }
  return entries;
}

/**
 * ACP usage_update: context tokens used / window size, and the session's cost when in USD.
 * `agent` is the guest's name; the session's own agent has none.
 */
function usageOf(update: Record<string, unknown>, agent: string | undefined): Usage | undefined {
  const usage: Usage = { type: "usage" };
  const used = count(update.used);
  const size = count(update.size);
  if (used !== undefined) usage.contextUsed = used;
  if (size !== undefined) usage.contextSize = size;
  if (isRecord(update.cost) && update.cost.currency === "USD") {
    const amount = update.cost.amount;
    if (typeof amount === "number" && Number.isFinite(amount) && amount >= 0) usage.cost = amount;
  }
  if (Object.keys(usage).length === 1) return undefined;
  if (agent !== undefined) usage.agent = agent;
  return usage;
}

/** Labels an update with the guest it is from; the session's own agent's stay unlabelled. */
function labelled<T extends object>(update: T, agent: string | undefined): T {
  return agent === undefined ? update : { ...update, agent };
}

/**
 * The ACP conversation a harness_message of this harness last named, if any since the context
 * was last cleared (/clear).
 */
export function storedConversation(history: SessionEvent[], name: string): string | undefined {
  let id: string | undefined;
  for (const event of history) {
    if (event.type === "context_cleared") id = undefined;
    if (event.type !== "harness_message" || event.harness !== name) continue;
    if (isRecord(event.message)) id = str(event.message.acpSession) ?? id;
  }
  return id;
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** ACP tool call content → ours (text and diffs; terminals are dropped). */
/** The local files an ACP tool call is about: its locations and the path fields of its input. */
function callPaths(call: Record<string, unknown>): string[] {
  const paths: string[] = [];
  if (Array.isArray(call.locations)) {
    for (const location of call.locations) {
      const path = isRecord(location) ? str(location.path) : undefined;
      if (path !== undefined) paths.push(path);
    }
  }
  if (isRecord(call.rawInput)) {
    for (const key of ["file_path", "path", "filePath", "notebook_path"]) {
      const path = str(call.rawInput[key]);
      if (path !== undefined) paths.push(path);
    }
  }
  return paths;
}

/** Whether the call starts one of the agent's own subagents (Claude Code's Agent or Task tool). */
function isSubagentStart(call: Record<string, unknown>): boolean {
  let tool = str(call.title);
  if (isRecord(call._meta) && isRecord(call._meta.claudeCode)) {
    tool = str(call._meta.claudeCode.toolName) ?? tool;
  }
  if (tool !== undefined && CLAUDE_AGENT_TOOLS.includes(tool)) return true;
  return isRecord(call.rawInput) && typeof call.rawInput.subagent_type === "string";
}

/** The shell command of an execute call (a string, or an argv list as Codex sends it). */
function callCommand(call: Record<string, unknown>): string | undefined {
  if (!isRecord(call.rawInput)) return undefined;
  const { command } = call.rawInput;
  if (!Array.isArray(command)) return str(command);
  return command.filter((c) => typeof c === "string").join(" ");
}

/** Keeps an image's bytes out of the log: base64 data and its mime type → an asset handle. */
export type StoreImage = (data: string, mimeType: string) => string | undefined;

function toContent(value: unknown, storeImage?: StoreImage): ToolCallContent[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const out: ToolCallContent[] = [];
  for (const item of value) {
    if (!isRecord(item)) continue;
    if (item.type === "diff" && typeof item.path === "string" && typeof item.newText === "string") {
      const diff: ToolCallContent = { type: "diff", path: item.path, newText: item.newText };
      if (typeof item.oldText === "string") diff.oldText = item.oldText;
      out.push(diff);
    } else if (item.type === "content" && isRecord(item.content)) {
      const text = str(item.content.text);
      const image = imageOf(item.content, storeImage);
      if (text !== undefined) out.push({ type: "text", text });
      else if (image) out.push(image);
      else out.push({ type: "text", text: `[${String(item.content.type)}]` });
    }
  }
  return out;
}

/** An ACP image block ({type: "image", data, mimeType} or a uri) as image content. */
function imageOf(
  block: Record<string, unknown>,
  storeImage?: StoreImage,
): ToolCallContent | undefined {
  if (block.type !== "image") return undefined;
  const mimeType = str(block.mimeType) ?? "image/png";
  const uri = str(block.uri);
  const data = str(block.data);
  const src = data === undefined || !storeImage ? uri : storeImage(data, mimeType);
  return src === undefined ? undefined : { type: "image", src, mimeType };
}

/**
 * Text of an MCP result an adapter put in rawOutput instead of content (Codex:
 * {result: {content}}), so [result:ref] reaches the view.
 */
export function mcpResultText(rawOutput: unknown): ToolCallContent[] | undefined {
  const result = isRecord(rawOutput) && isRecord(rawOutput.result) ? rawOutput.result : rawOutput;
  if (!isRecord(result) || !Array.isArray(result.content)) return undefined;
  const texts = result.content.filter(isRecord).map((block) => str(block.text));
  const text = texts.filter((t) => t !== undefined).join("\n");
  return text === "" ? undefined : [{ type: "text", text }];
}

/**
 * Neo's core tools as an adapter names them (Claude Code: mcp__neo__tools_call with the
 * arguments as input; Codex: mcp.neo.tools_call with {server, tool, arguments}): show what
 * they do instead.
 */
export function describeCall(title: string, rawInput: unknown): { title: string; tool?: string } {
  const plain = title.replace(/^mcp(__|\.)neo(__|\.)/, "");
  // The view draws a delegation by its arguments (agent, task).
  if (plain === "delegate") return { title: plain, tool: plain };
  const named = plain === "wait" ? "wait for messages" : plain;
  if (!isRecord(rawInput)) return { title: named };
  const args = isRecord(rawInput.arguments) ? rawInput.arguments : rawInput;
  if (plain.includes("tools_call")) {
    const name = str(args.name);
    if (name !== undefined) return { title: name, tool: name };
  } else if (plain.includes("tools_search")) {
    const query = str(args.query);
    if (query !== undefined) return { title: `search tools: ${query}` };
  } else if (plain === "send_message") {
    const to = str(args.to);
    const first = (str(args.text) ?? "").split("\n")[0] ?? "";
    if (to !== undefined) return { title: `message ${to}: ${first.slice(0, 80)}` };
  }
  return { title: named };
}

/**
 * Claude Code's NotebookEdit as an edit with something to show. Its adapter describes the call
 * only to JetBrains AIR; anyone else gets the bare title `NotebookEdit`, kind `other` and no
 * content. The input has it all: the notebook, the cell, insert/replace/delete and the new
 * source, which goes in as the content until the call's own result comes.
 */
export function withNotebookEdit(call: Record<string, unknown>): Record<string, unknown> {
  const tool =
    isRecord(call._meta) && isRecord(call._meta.claudeCode)
      ? str(call._meta.claudeCode.toolName)
      : undefined;
  if (tool !== "NotebookEdit" && call.title !== "NotebookEdit") return call;
  const out: Record<string, unknown> = { ...call, kind: "edit" };
  const input = isRecord(call.rawInput) ? call.rawInput : undefined;
  const path = str(input?.notebook_path);
  if (path === undefined) return out;
  const mode = str(input?.edit_mode);
  const verbs: Record<string, string> = { insert: "Insert", delete: "Delete" };
  const words = [verbs[mode ?? ""] ?? "Edit", "cell"];
  const cell = str(input?.cell_id);
  if (cell !== undefined) words.push(cell);
  out.title = `${words.join(" ")} in ${path}`;
  out.locations = [{ path }];
  const source = str(input?.new_source);
  const empty = !Array.isArray(call.content) || call.content.length === 0;
  if (mode !== "delete" && source !== undefined && empty) {
    out.content = [{ type: "content", content: { type: "text", text: source } }];
  }
  return out;
}

/**
 * Whether a permission request is for one of neo's own MCP tools (tools_search, tools_call,
 * apps_launch, apps_focus, delegate, models, send_message, wait), as Claude Code
 * (`mcp__neo__tools_call`, also in its `_meta.claudeCode.toolName`) and Codex
 * (`mcp.neo.tools_call`, or the server and tool in the input) name them. The gateway gates what needs gating, so these are not asked about twice.
 */
export function isNeoTool(call: Record<string, unknown>): boolean {
  const named = (title: string | undefined): boolean => {
    const match = title === undefined ? null : /^mcp(?:__|\.)neo(?:__|\.)(\w+)$/.exec(title);
    return match?.[1] !== undefined && isCoreToolName(match[1]);
  };
  const claude =
    isRecord(call._meta) && isRecord(call._meta.claudeCode)
      ? str(call._meta.claudeCode.toolName)
      : undefined;
  if (named(str(call.title)) || named(claude)) return true;
  const input = call.rawInput;
  const tool = isRecord(input) ? str(input.tool) : undefined;
  return isRecord(input) && input.server === "neo" && tool !== undefined && isCoreToolName(tool);
}

/** ACP config options (flat or grouped selects) → ours, keyed by config id. */
export function configOptionsOf(value: unknown): Map<string, ConfigOption> {
  const out = new Map<string, ConfigOption>();
  if (!Array.isArray(value)) return out;
  for (const option of value.filter(isRecord)) {
    const id = str(option.id);
    const category = str(option.category);
    if (
      id === undefined ||
      (category !== "model" && category !== "thought_level" && category !== "mode")
    ) {
      continue;
    }
    const values: ConfigOption["values"] = [];
    // Options may come grouped ({group, options: […]}).
    const listed: unknown[] = [];
    if (Array.isArray(option.options)) listed.push(...option.options);
    const flat: unknown[] = [];
    for (const o of listed) {
      if (isRecord(o) && Array.isArray(o.options)) flat.push(...o.options);
      else flat.push(o);
    }
    for (const entry of flat.filter(isRecord)) {
      const v = str(entry.value);
      if (v === undefined) continue;
      const item: ConfigOption["values"][number] = { value: v, name: str(entry.name) ?? v };
      const description = str(entry.description);
      if (description !== undefined) item.description = description;
      values.push(item);
    }
    const parsed: ConfigOption = { category, values };
    const current = str(option.currentValue);
    if (current !== undefined) parsed.current = current;
    out.set(id, parsed);
  }
  return out;
}

async function imageBlocks(paths: string[] | undefined): Promise<Record<string, unknown>[]> {
  const blocks: Record<string, unknown>[] = [];
  for (const path of paths ?? []) {
    const mimeType = IMAGE_TYPES[extname(path).toLowerCase()];
    if (mimeType === undefined) continue;
    const data = Buffer.from(await Bun.file(path).arrayBuffer()).toString("base64");
    blocks.push({ type: "image", mimeType, data });
  }
  return blocks;
}

export class GuestManager {
  private readonly guests = new Map<string, Promise<Guest>>();
  /** Every running adapter (guests and sessions' own agents), to stop them with the daemon. */
  private readonly live = new Set<Guest>();
  /** What the user wrote in native subagents' sessions, waiting to be passed on, by session. */
  private readonly relays = new Map<string, { parent: string; at: number }>();
  private readonly subagents?: NeoSubagents;
  /** The commands each session's guest last listed, as JSON, so an unchanged list isn't logged again. */
  private readonly commandLists = new Map<string, string>();
  /** Sessions that were deleted: an adapter still starting for one is stopped once it is up. */
  private readonly forgotten = new Set<string>();

  constructor(private readonly options: GuestManagerOptions) {
    const { neo, requestPermission } = options;
    if (!neo) return;
    const subagents: SubagentOptions = {
      log: options.log,
      cwd: options.cwd,
      // A guest of the gateway (no delegate), but one that sees structured results.
      mcpServers: (session) => options.mcpServers(session, { name: NEO, guest: true }),
      settings: (session) => options.settings(session, NEO),
      connectors: options.connectors,
      focus: options.focus,
    };
    if (options.showDontTell) subagents.showDontTell = options.showDontTell;
    if (options.resolveAsset) subagents.resolveAsset = options.resolveAsset;
    if (requestPermission)
      subagents.requestPermission = (session) => requestPermission(session, NEO);
    this.subagents = new NeoSubagents(neo, subagents);
  }

  names(): string[] {
    return Object.keys(this.options.configs);
  }

  has(name: string): boolean {
    return Object.hasOwn(this.options.configs, name);
  }

  /** Everything delegate can hand a task to: neo (when there is one) and the configured guests. */
  harnesses(): string[] {
    return this.subagents ? [NEO, ...this.names()] : this.names();
  }

  title(name: string): string {
    return this.options.configs[name]?.title ?? name;
  }

  /**
   * Whether the guest can use subagents of its own (see GuestConfig.subagents): ones that start
   * without asking not while the user wants every start asked about.
   */
  ownSubagents(name: string): boolean {
    const kind = this.options.configs[name]?.subagents;
    if (kind === "silent") return this.options.alwaysAskForAgents?.() !== true;
    return kind === "asks";
  }

  /**
   * The agent definitions (neo's, see harness agents.ts) the session can delegate to by name: the
   * project's, the user's and the plugins'.
   */
  agentDefinitions(session: string): AgentInfo[] {
    if (!this.subagents) return [];
    try {
      return this.options.neo?.agents?.(this.options.cwd(session)) ?? [];
    } catch (err) {
      this.options.log.warn(`reading the agent definitions failed: ${errorText(err)}`);
      return [];
    }
  }

  /**
   * Runs one task. Updates go to `emit` as they come; the result carries the guest's last
   * message. Never rejects: failures resolve with stopReason "error".
   */
  async run(
    session: string,
    name: string,
    task: string,
    opts: RunOptions,
  ): Promise<GuestRunResult> {
    if (name === NEO && this.subagents) return this.subagents.run(session, task, opts);
    if (!this.has(name)) {
      return { stopReason: "error", error: `No guest named ${name}.`, text: "" };
    }
    let guest: Guest;
    try {
      guest = await this.guest(session, name);
    } catch (err) {
      return { stopReason: "error", error: `Starting ${name} failed: ${errorText(err)}`, text: "" };
    }
    return this.enqueue(guest, task, opts);
  }

  /** One prompt at a time per adapter. */
  private enqueue(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const turn = guest.queue.then(() => this.runWith(guest, task, opts));
    guest.queue = turn.catch(() => {});
    return turn;
  }

  /**
   * Runs the task with the settings that are meant for it alone (delegate's model and effort),
   * then puts the previous values back. A value the adapter refuses ends the task with its
   * message.
   */
  private async runWith(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    const before: [ConfigCategory, string][] = [];
    const back = async (): Promise<void> => {
      for (const [category, value] of before) {
        await this.setOption(guest, category, value).catch((err: unknown) => {
          this.options.log.warn(
            `${guest.name}: putting ${category} back failed: ${errorText(err)}`,
          );
        });
      }
    };
    for (const [name, value] of Object.entries(opts.settings ?? {})) {
      const category = name as ConfigCategory;
      const current = [...guest.config.values()].find((o) => o.category === category)?.current;
      try {
        await this.setOption(guest, category, value);
      } catch (err) {
        await back();
        return { stopReason: "error", error: errorText(err), text: "" };
      }
      if (current !== undefined && current !== value) before.push([category, current]);
    }
    try {
      return await this.prompt(guest, task, opts);
    } finally {
      await back();
    }
  }

  /** The guest as an AgentSession, for turns the user gives it directly (/claude task). */
  turnSession(session: string, name: string): AgentSession | undefined {
    if (!this.has(name)) return undefined;
    let controller: AbortController | undefined;
    return {
      prompt: async (params) => {
        controller = new AbortController();
        const opts = { emit: params.onUpdate, signal: controller.signal, images: params.images };
        const result = await this.run(session, name, params.text, opts);
        // The main agent hears about it with its next prompt.
        this.options.append(session, {
          type: "guest_turn",
          agent: name,
          task: params.text,
          result: result.text,
          stopReason: result.stopReason,
        });
        const out: PromptResult = { stopReason: result.stopReason };
        if (result.error !== undefined) out.error = result.error;
        return out;
      },
      cancel: () => controller?.abort(),
    };
  }

  /**
   * The harness as the session's own agent (the user picked it with /harness): sessions create
   * and load it like neo. Its conversation is the adapter's own, named in the log
   * (harness_message {acpSession}) so a restarted daemon resumes it.
   */
  agent(name: string): Agent {
    return {
      name,
      newSession: (params) => this.mainSession(params.sessionId, name, undefined),
      loadSession: (params) =>
        this.mainSession(params.sessionId, name, storedConversation(params.history, name)),
    };
  }

  private async mainSession(
    session: string,
    name: string,
    stored: string | undefined,
  ): Promise<AgentSession> {
    let guest = await this.start(session, name, { main: true, resume: stored });
    let controller: AbortController | undefined;
    return {
      prompt: async (params) => {
        const abort = new AbortController();
        controller = abort;
        try {
          // The adapter may have died since the last turn: continue its conversation.
          if (!guest.conn.running) {
            guest = await this.start(session, name, { main: true, resume: guest.acpSession });
          }
        } catch (err) {
          return { stopReason: "error", error: `Starting ${name} failed: ${errorText(err)}` };
        }
        const current = guest;
        // A conversation it doesn't carry gets everything said so far, not just what it missed.
        const summary = current.fresh
          ? this.options.handoff?.(session, name, true)
          : params.summary;
        current.fresh = false;
        const opts: RunOptions = {
          emit: params.onUpdate,
          signal: abort.signal,
          images: params.images,
          events: params.pendingEvents,
        };
        if (params.takeMessages) opts.takeMessages = params.takeMessages;
        if (summary !== undefined) opts.summary = summary;
        const result = await this.enqueue(current, params.text, opts);
        const out: PromptResult = { stopReason: result.stopReason };
        if (result.error !== undefined) out.error = result.error;
        return out;
      },
      cancel: () => controller?.abort(),
      steer: async ({ text, images }) => {
        if (!guest.steering || guest.run === undefined) return false;
        try {
          const reply = await guest.conn.request(STEER_METHOD, {
            sessionId: guest.acpSession,
            prompt: [{ type: "text", text }, ...(await imageBlocks(images))],
            // No turn running: say so, and the daemon starts one.
            _meta: { steering: { idleBehavior: "promptRequired" } },
          });
          return isRecord(reply) && reply.outcome === "injected";
        } catch (err) {
          this.options.log.warn(`${name}: steering failed: ${errorText(err)}`);
          return false;
        }
      },
      background: async (toolCallId) => {
        if (!guest.backgroundTasks || guest.run === undefined) return false;
        // The log names the session agent's calls `<name>-main:<id>`; the adapter knows the id.
        const own = `${name}-main:`;
        if (!toolCallId.startsWith(own)) return false;
        try {
          const reply = await guest.conn.request(BACKGROUND_METHOD, {
            sessionId: guest.acpSession,
            toolCallId: toolCallId.slice(own.length),
          });
          return isRecord(reply) && reply.backgrounded === true;
        } catch (err) {
          this.options.log.warn(`${name}: sending to the background failed: ${errorText(err)}`);
          return false;
        }
      },
      configOptions: () => [...guest.config.values()],
      loadConfigOptions: () => this.optionsOf(guest),
      setConfigOption: async (category, value) => {
        guest = await this.configureGuest(guest, category, value);
      },
    };
  }

  /**
   * The options (models, effort levels) a harness offers, when they can be had without starting
   * anything: neo's own, or those of an adapter that already runs. undefined: not known yet.
   */
  async knownOptions(session: string, name: string): Promise<ConfigOption[] | undefined> {
    if (name === NEO && this.subagents) return this.subagents.configOptions(session);
    for (const guest of this.live) {
      if (guest.name === name) return [...guest.config.values()];
    }
    return undefined;
  }

  /** The guest's settable options (starts it for this session if it isn't running). */
  async configOptions(session: string, name: string): Promise<ConfigOption[]> {
    return this.optionsOf(await this.guest(session, name));
  }

  /**
   * The guest's options, with neo's accounts added to its models (a guest with a `route`):
   * `provider/model` values that start it on that key.
   */
  private async optionsOf(guest: Guest): Promise<ConfigOption[]> {
    const options = [...guest.config.values()];
    const kind = this.options.configs[guest.name]?.route;
    const { accounts } = this.options;
    if (kind === undefined || !accounts) return options;
    const extra = await accounts.models(kind);
    return options.map((option) => {
      if (option.category !== "model") return option;
      const known = new Set(option.values.map((v) => v.value));
      const values = [...option.values, ...extra.filter((m) => !known.has(m.value))];
      const out: ConfigOption = { ...option, values };
      if (guest.route !== undefined) out.current = guest.route.id;
      return out;
    });
  }

  /**
   * Sets an option on this session's guest (starting it if needed). Rejects with the adapter's
   * message when it doesn't take the value.
   */
  async configure(
    session: string,
    name: string,
    category: ConfigCategory,
    value: string,
  ): Promise<void> {
    const key = `${session}\0${name}`;
    const guest = await this.guest(session, name);
    const changed = await this.configureGuest(guest, category, value);
    if (changed !== guest) this.guests.set(key, Promise.resolve(changed));
  }

  /**
   * Sets an option. A model that is `provider/model` of a neo account, or a plain model after
   * one, restarts the adapter on the other key (continuing its conversation); the guest it
   * returns is the one to keep using.
   */
  private async configureGuest(
    guest: Guest,
    category: ConfigCategory,
    value: string,
  ): Promise<Guest> {
    const kind = this.options.configs[guest.name]?.route;
    const { accounts } = this.options;
    if (category !== "model" || kind === undefined || !accounts) {
      await this.setOption(guest, category, value);
      return guest;
    }
    const route = await accounts.route(kind, value);
    if (route === undefined && guest.route === undefined) {
      await this.setOption(guest, category, value);
      return guest;
    }
    const next = route?.id === guest.route?.id ? guest : await this.reroute(guest, route);
    if (route === undefined) await this.setOption(next, category, value);
    return next;
  }

  /** Starts the adapter again on another key (or its own login), in the same conversation. */
  private async reroute(guest: Guest, route: Route | undefined): Promise<Guest> {
    // A turn that is running finishes first.
    await guest.queue;
    this.live.delete(guest);
    guest.conn.kill();
    return this.start(guest.session, guest.name, {
      main: guest.main,
      resume: guest.acpSession,
      route: route ?? null,
      lenient: true,
    });
  }

  /** Conversations the guest has stored for this directory, newest first (session/list). */
  async list(name: string, cwd: string): Promise<StoredConversation[]> {
    const conn = this.spawn(name, cwd, {});
    try {
      await this.initialize(conn);
      const listed = await conn.request("session/list", { cwd });
      const out: StoredConversation[] = [];
      if (!isRecord(listed) || !Array.isArray(listed.sessions)) return out;
      const sessions: unknown[] = listed.sessions;
      for (const entry of sessions.filter(isRecord)) {
        const id = str(entry.sessionId);
        if (id === undefined) continue;
        const item: StoredConversation = { id };
        const title = str(entry.title);
        const updatedAt = str(entry.updatedAt);
        if (title !== undefined) item.title = title;
        if (updatedAt !== undefined) item.updatedAt = updatedAt;
        out.push(item);
      }
      return out;
    } finally {
      conn.kill();
    }
  }

  /**
   * One prompt to the guest's adapter outside any session, for short generated text: a throwaway
   * adapter process and conversation in an empty temporary directory, nothing allowed (a tool
   * asking is turned down) and nothing logged, ended once the reply is in. Resolves with the
   * agent's last message; rejects with a message the user can read.
   */
  async generateText(name: string, params: GenerateTextParams): Promise<string> {
    const config = this.options.configs[name];
    if (!config) throw new BadRequestError(`no guest named ${name}`);
    const title = this.title(name);
    const cwd = await mkdtemp(join(tmpdir(), "neo-text-"));
    const messages = new Map<string, string>();
    const conn = AcpConnection.spawn({
      command: config.command,
      cwd,
      env: config.env,
      onNotification: (method, update) => {
        if (method !== "session/update" || !isRecord(update) || !isRecord(update.update)) return;
        const chunk = update.update;
        if (chunk.sessionUpdate !== "agent_message_chunk" || !isRecord(chunk.content)) return;
        const text = str(chunk.content.text);
        const id = str(chunk.messageId) ?? "";
        if (text !== undefined) messages.set(id, (messages.get(id) ?? "") + text);
      },
      onRequest: (method) => {
        if (method === "session/request_permission") {
          return Promise.resolve({ outcome: { outcome: "cancelled" } });
        }
        return Promise.reject(new AcpError(`${method} is not supported`, -32601));
      },
    });
    let acpSession: string | undefined;
    const cancel = (): void => {
      if (acpSession !== undefined) conn.notify("session/cancel", { sessionId: acpSession });
    };
    params.signal?.addEventListener("abort", cancel);
    try {
      await this.initialize(conn);
      const created = await conn.request("session/new", { cwd, mcpServers: [] });
      acpSession = isRecord(created) ? str(created.sessionId) : undefined;
      if (acpSession === undefined) throw new Error("session/new returned no sessionId");
      if (params.model !== undefined) {
        const options = configOptionsOf(isRecord(created) ? created.configOptions : undefined);
        const entry = [...options].find(([, option]) => option.category === "model");
        if (!entry) throw new BadRequestError(`${title} has no model setting`);
        await conn.request("session/set_config_option", {
          sessionId: acpSession,
          configId: entry[0],
          value: params.model,
        });
      }
      await conn.request("session/prompt", {
        sessionId: acpSession,
        prompt: [{ type: "text", text: `${params.system}\n\n${params.prompt}` }],
      });
      return [...messages.values()].join("").trim();
    } catch (err) {
      throw new Error(`${title}: ${errorText(err)}`);
    } finally {
      params.signal?.removeEventListener("abort", cancel);
      conn.kill();
      await rm(cwd, { recursive: true, force: true });
    }
  }

  /** Continues a stored conversation as this session's guest, replacing the running one. */
  async resume(session: string, name: string, conversation: string): Promise<void> {
    const key = `${session}\0${name}`;
    const running = this.guests.get(key);
    this.guests.delete(key);
    if (running) {
      const guest = await running.catch(() => undefined);
      if (guest) {
        this.live.delete(guest);
        guest.conn.kill();
      }
    }
    const resumed = this.start(session, name, { resume: conversation });
    this.guests.set(key, resumed);
    resumed.catch(() => this.guests.delete(key));
    await resumed;
  }

  /** Stops the adapters of a session that was deleted and forgets what was kept for it. */
  forget(session: string): void {
    this.forgotten.add(session);
    this.stop(session);
  }

  /**
   * Stops the adapters of a session (it moved to another directory): the next guest it addresses
   * starts over, where it works now.
   */
  stop(session: string): void {
    for (const guest of this.live) {
      if (guest.session === session) {
        this.live.delete(guest);
        guest.conn.kill();
      }
    }
    const prefix = `${session}\0`;
    for (const key of [...this.guests.keys()]) {
      if (key.startsWith(prefix)) this.guests.delete(key);
    }
    for (const key of [...this.commandLists.keys()]) {
      if (key.startsWith(prefix)) this.commandLists.delete(key);
    }
  }

  shutdown(): void {
    for (const guest of this.live) guest.conn.kill();
    this.guests.clear();
    this.live.clear();
  }

  private guest(session: string, name: string): Promise<Guest> {
    if (!this.has(name)) return Promise.reject(new BadRequestError(`no guest named ${name}`));
    const key = `${session}\0${name}`;
    let guest = this.guests.get(key);
    if (!guest) {
      guest = this.start(session, name, {});
      this.guests.set(key, guest);
      guest.catch(() => this.guests.delete(key));
    }
    return guest;
  }

  /**
   * `target` gets the guest once it exists: the connection's updates and requests are its. What
   * the adapter lists before that (its commands) is kept in `target` for the guest.
   */
  private spawn(
    name: string,
    cwd: string,
    target: GuestTarget,
    route?: Route,
    native = false,
  ): AcpConnection {
    const config = this.options.configs[name];
    if (!config) throw new NotFoundError(`no guest ${name}`);
    const env = route ? { ...config.env, ...route.env } : { ...config.env };
    if (env.CODEX_CONFIG !== undefined) {
      env.CODEX_CONFIG = withCodexSubagents(env.CODEX_CONFIG, native);
    }
    return AcpConnection.spawn({
      command: config.command,
      cwd,
      env,
      onNotification: (method, params) => this.onNotification(target, method, params),
      onRequest: (method, params) => this.onRequest(target.guest, method, params),
    });
  }

  /**
   * Remembers the commands the adapter lists and puts them in the session's log, for views to
   * offer in their / menu. Only for the session's own agent (a subagent's aren't the user's), and
   * only when they changed.
   */
  private setCommands(guest: Guest, commands: SlashCommand[]): void {
    guest.commands = commands;
    if (!guest.main) return;
    const key = `${guest.session}\0${guest.name}`;
    const json = JSON.stringify(commands);
    if (this.commandLists.get(key) === json) return;
    this.commandLists.set(key, json);
    this.options.append(guest.session, {
      type: "commands_available",
      harness: guest.name,
      commands,
    });
  }

  /**
   * Says hello; resolves with whether the adapter can be steered mid-turn, and whether it can send
   * a running tool call's task to the background.
   */
  private async initialize(
    conn: AcpConnection,
    route?: Route,
  ): Promise<{ steering: boolean; backgroundTasks: boolean }> {
    const clientCapabilities: Record<string, unknown> = {
      fs: { readTextFile: false, writeTextFile: false },
      terminal: false,
      // The harness's own subagents report as ACP sessions of their own (see NativeChild).
      subagents: {},
    };
    // Codex's adapter takes a custom gateway only from a client that says it can sign in to one.
    if (route?.gateway) clientCapabilities.auth = { _meta: { gateway: true } };
    const hello = await conn.request("initialize", {
      protocolVersion: 1,
      clientCapabilities,
      clientInfo: { name: "neo", version: "0.6.0" },
    });
    if (route?.gateway) {
      await conn.request("authenticate", {
        methodId: "gateway",
        _meta: { gateway: route.gateway },
      });
    }
    const meta = isRecord(hello) && isRecord(hello._meta) ? hello._meta : undefined;
    const supported = (key: string): boolean => {
      const entry = meta?.[key];
      return isRecord(entry) && entry.supported === true;
    };
    return { steering: supported("steering"), backgroundTasks: supported("backgroundTasks") };
  }

  /**
   * Starts the adapter and a new conversation, or resumes a stored one. The session's own agent
   * falls back to a new conversation when the stored one can't be resumed.
   */
  private async start(
    session: string,
    name: string,
    opts: {
      main?: boolean;
      resume?: string;
      /** The key to run on; null: the guest's own login; unset: what the session's model says. */
      route?: Route | null;
      /** Start a new conversation when the stored one can't be resumed (default: for main). */
      lenient?: boolean;
    },
  ): Promise<Guest> {
    const { main = false, resume, route } = opts;
    const { lenient = main } = opts;
    if (lenient && resume !== undefined) {
      try {
        return await this.open(session, name, main, resume, route);
      } catch (err) {
        this.options.log.warn(
          `${name} for ${session}: resuming ${resume} failed (${errorText(err)}); starting anew`,
        );
      }
      return this.open(session, name, main, undefined, route);
    }
    return this.open(session, name, main, resume, route);
  }

  /** The key the session's model setting names, if any (the guest is restarting or new). */
  private async routeFromSettings(session: string, name: string): Promise<Route | undefined> {
    const kind = this.options.configs[name]?.route;
    const model = this.options.settings(session, name).model;
    if (kind === undefined || model === undefined || !this.options.accounts) return undefined;
    try {
      return await this.options.accounts.route(kind, model);
    } catch (err) {
      this.options.log.warn(`${name} for ${session}: ${errorText(err)}; using its own login`);
      return undefined;
    }
  }

  private async open(
    session: string,
    name: string,
    main: boolean,
    resume: string | undefined,
    chosen?: Route | null,
  ): Promise<Guest> {
    const { log } = this.options;
    const cwd = this.options.cwd(session);
    const target: GuestTarget = {};
    const route = chosen === undefined ? await this.routeFromSettings(session, name) : chosen;
    const native =
      main && this.ownSubagents(name) && this.options.nativeSubagents?.(session) === true;
    const conn = this.spawn(name, cwd, target, route ?? undefined, native);
    void conn.exited.then((code) => {
      log.info(`${name} for ${session} exited (${code})`);
      const { guest } = target;
      if (!guest) return;
      this.live.delete(guest);
      for (const child of guest.children.values()) {
        this.endChild(guest, child, "error", `${this.title(name)} exited`);
      }
      if (guest.own) this.endOwnTurn(guest.own, "error");
      if (main) return;
      const key = `${session}\0${name}`;
      // Only if it's still the current one (a resume replaces it).
      void this.guests.get(key)?.then((current) => current === guest && this.guests.delete(key));
    });
    try {
      const { steering, backgroundTasks } = await this.initialize(conn, route ?? undefined);
      const mcpServers = this.options
        .mcpServers(session, { name, guest: !main, text: true })
        .map((server) => ({ type: "http", ...server }));
      const params: Record<string, unknown> = {
        cwd,
        mcpServers,
        _meta: {
          // Both adapters append this to their own system prompt.
          jetbrains: { air: { customInstructions: main ? MAIN_PROMPT : GUEST_PROMPT } },
          // Claude Code's own subagents, when the user lets the agent use them, ask before they
          // start like any tool (onRequest decides); else they are off and delegate hands work
          // on. (Codex: CODEX_CONFIG, see spawn.)
          claudeCode: {
            options: native
              ? { settings: { permissions: { ask: CLAUDE_AGENT_TOOLS } } }
              : { disallowedTools: CLAUDE_AGENT_TOOLS },
          },
        },
      };
      if (resume !== undefined) params.sessionId = resume;
      const created = await conn.request(
        resume === undefined ? "session/new" : "session/resume",
        params,
      );
      const acpSession = resume ?? (isRecord(created) ? str(created.sessionId) : undefined);
      if (acpSession === undefined) throw new Error("session/new returned no sessionId");
      const config = configOptionsOf(isRecord(created) ? created.configOptions : undefined);
      const guest: Guest = {
        name,
        session,
        main,
        fresh: main && resume === undefined,
        conn,
        acpSession,
        told: new ToldState(),
        queue: Promise.resolve(),
        config,
        steering,
        backgroundTasks,
        children: new Map(),
        backgroundCalls: new Map(),
      };
      if (route) guest.route = route;
      if (this.forgotten.has(session)) {
        throw new NotFoundError(`no session ${session}`);
      }
      target.guest = guest;
      this.live.add(guest);
      if (target.commands) this.setCommands(guest, target.commands);
      // Only what the user chose: everything else stays the guest's own default.
      for (const [category, value] of Object.entries(this.options.settings(session, name))) {
        // The model of a key was given at launch.
        if (route && category === "model") continue;
        try {
          await this.setOption(guest, category as ConfigCategory, value);
        } catch (err) {
          log.warn(`${name}: setting ${category} to ${value} failed: ${errorText(err)}`);
        }
      }
      if (main && resume === undefined) {
        // So a restarted daemon continues this conversation.
        this.options.append(session, {
          type: "harness_message",
          harness: name,
          message: { acpSession },
        });
      }
      log.info(
        `${name} ${resume === undefined ? "started" : "resumed"} for ${session} (${acpSession})`,
      );
      return guest;
    } catch (err) {
      conn.kill();
      if (!route) throw err;
      // What the adapter says may quote the request that carried the key.
      let text = errorText(err);
      for (const secret of route.secrets) text = text.replaceAll(secret, "***");
      throw new Error(text);
    }
  }

  private async setOption(guest: Guest, category: ConfigCategory, value: string): Promise<void> {
    const entry = [...guest.config].find(([, option]) => option.category === category);
    if (!entry) {
      const name = category === "thought_level" ? "effort" : category;
      throw new BadRequestError(`${this.title(guest.name)} has no ${name} setting`);
    }
    const [configId, option] = entry;
    try {
      const result = await guest.conn.request("session/set_config_option", {
        sessionId: guest.acpSession,
        configId,
        value,
      });
      const updated = configOptionsOf(isRecord(result) ? result.configOptions : undefined);
      if (updated.size > 0) guest.config = updated;
      else option.current = value;
    } catch (err) {
      throw new BadRequestError(`${this.title(guest.name)}: ${errorText(err)}`);
    }
  }

  private async prompt(guest: Guest, task: string, opts: RunOptions): Promise<GuestRunResult> {
    // The session's own agent is unlabelled; ids are prefixed so they stay unique in the log.
    const run: Run = {
      emit: opts.emit,
      prefix: guest.main ? `${guest.name}-main` : guest.name,
      messages: new Map(),
      done: new AbortController(),
      titles: new Map(),
    };
    if (!guest.main) run.agent = guest.name;
    const text = (): string => [...run.messages.values()].at(-1)?.trim() ?? "";
    if (opts.signal?.aborted) return { stopReason: "cancelled", text: "" };
    // What the session wants the agent to hear (guests' turns, reverted edits, the user's notes),
    // with what an earlier command's prompt couldn't carry.
    const carried = guest.carried;
    guest.carried = undefined;
    const said = [...(carried?.events ?? [])];
    for (const event of opts.events ?? []) {
      if (["guest_turn", "edits_reverted", "user_note"].includes(event.type)) said.push(event);
    }
    const summary = opts.summary ?? carried?.summary;
    // A slash command goes to the adapter alone, or it isn't one (`/compact`); the rest waits.
    const command = task.trimStart().startsWith("/");
    if (command) {
      guest.carried = { events: said };
      if (summary !== undefined) guest.carried.summary = summary;
    }
    // What this adapter wasn't told yet about the connectors and focus (told next time, for a
    // command).
    const events: ContextEvent[] = [];
    if (!command) {
      events.push(
        ...guest.told.changes(
          this.options.connectors(),
          this.options.focus(guest.session),
          this.options.showDontTell?.(),
        ),
      );
      for (const event of events) guest.told.apply(event);
      events.push(...said);
    }
    const onAbort = (): void => {
      guest.conn.notify("session/cancel", { sessionId: guest.acpSession });
      run.done.abort();
    };
    opts.signal?.addEventListener("abort", onAbort);
    guest.run = run;
    const began = Date.now();
    try {
      // What arrived before this turn, ahead of the task (the gateway adds what comes during the
      // turn to its tool results).
      let body = task.trim();
      if (!command) {
        const waiting = opts.takeMessages?.() ?? [];
        body = [...waiting, formatPrompt(task, events, undefined, summary)].join("\n\n");
      }
      const prompt = [{ type: "text", text: body }, ...(await imageBlocks(opts.images))];
      const result = await guest.conn.request("session/prompt", {
        sessionId: guest.acpSession,
        prompt,
      });
      const reason = isRecord(result) ? str(result.stopReason) : undefined;
      const stopReason: StopReason =
        reason !== undefined && STOP_REASONS.has(reason) ? (reason as StopReason) : "end_turn";
      return { stopReason, text: text() };
    } catch (err) {
      if (opts.signal?.aborted) return { stopReason: "cancelled", text: text() };
      return { stopReason: "error", error: `${guest.name}: ${errorText(err)}`, text: text() };
    } finally {
      opts.signal?.removeEventListener("abort", onAbort);
      run.done.abort();
      guest.run = undefined;
      if (guest.main && !command) this.relaysLeft(guest, began);
    }
  }

  private onNotification(target: GuestTarget, method: string, params: unknown): void {
    if (method !== "session/update" || !isRecord(params) || !isRecord(params.update)) return;
    const { guest } = target;
    // Outside any turn, and perhaps before session/new has answered.
    if (params.update.sessionUpdate === "available_commands_update") {
      const commands = slashCommands(params.update.availableCommands);
      if (guest) this.setCommands(guest, commands);
      else target.commands = commands;
      return;
    }
    // (A resumed conversation's history comes before there is a guest: it is in the log already.)
    if (!guest) return;
    const update = params.update;
    const sessionId = str(params.sessionId);
    if (this.nativeLifecycle(guest, update)) return;
    if (sessionId !== undefined && sessionId !== guest.acpSession) {
      const child = guest.children.get(sessionId);
      // The prompt opens its transcript, as delegate's task does.
      if (child && update.sessionUpdate === "session_message") {
        const content = Array.isArray(update.content) ? update.content : [];
        const task = content
          .map((c: unknown) => (isRecord(c) ? (str(c.text) ?? "") : ""))
          .join("")
          .trim();
        // Its id is that of the Agent call that started it (or of the SendMessage that resumed it).
        const call = str(update.messageId);
        if (call !== undefined && guest.backgroundCalls.get(call)) child.background = true;
        if (call !== undefined) guest.backgroundCalls.delete(call);
        if (child.running && !child.started && task !== "") this.startChild(guest, child, task);
        return;
      }
      if (child?.running) {
        if (!child.started) this.startChild(guest, child);
        this.streamUpdate(child.stream, update);
      }
      return;
    }
    const run = guest.run;
    if (!run) {
      this.holdOwnTurn(guest, update);
      return;
    }
    // Whether an Agent call runs in the background, for the subagent it starts (its input may
    // arrive with an update of the call).
    const call = str(update.toolCallId);
    if (call !== undefined && isRecord(update.rawInput) && update.rawInput.subagent_type) {
      guest.backgroundCalls.set(call, update.rawInput.run_in_background === true);
    }
    this.streamUpdate(run, update);
    const own = guest.own;
    if (own?.run === run) this.ownTurnGoesOn(guest, own, update);
  }

  /**
   * An update of the session's agent while no turn runs: the start or more of work it does on
   * its own (see OwnTurn), held for the turn the session runs for it.
   */
  private holdOwnTurn(guest: Guest, update: Record<string, unknown>): void {
    if (!guest.main || !this.options.ownTurn) return;
    const own = guest.own;
    if (own) {
      // More of it before its turn started; one that ended meanwhile goes on.
      delete own.stopReason;
      own.held.push(update);
      this.ownTurnGoesOn(guest, own, update);
      return;
    }
    if (!OWN_TURN_STARTS.has(String(update.sessionUpdate))) return;
    const started: OwnTurn = { held: [update] };
    guest.own = started;
    // (Its turn may start at once, and take what is held.)
    this.options.ownTurn(guest.session, this.title(guest.name));
    this.ownTurnGoesOn(guest, started, update);
  }

  /** The agent's own work sent this: its result's usage ends it, else it ends when quiet. */
  private ownTurnGoesOn(guest: Guest, own: OwnTurn, update: Record<string, unknown>): void {
    clearTimeout(own.quiet);
    if (update.sessionUpdate === "usage_update" && update.cost !== undefined) {
      this.endOwnTurn(own, "end_turn");
      return;
    }
    own.quiet = setTimeout(() => this.endOwnTurn(own, "end_turn"), OWN_TURN_QUIET_MS);
    own.quiet.unref?.();
  }

  private endOwnTurn(own: OwnTurn, stopReason: StopReason): void {
    clearTimeout(own.quiet);
    if (own.stopReason !== undefined) return;
    own.stopReason = stopReason;
    own.end?.();
  }

  /**
   * The turn of the session that shows what its agent does on its own (see OwnTurn): its
   * prompt() streams that work into the log and resolves when it is over. Undefined when there
   * is none (any more).
   */
  ownTurnSession(session: string): AgentSession | undefined {
    const guest = [...this.live].find((g) => g.main && g.session === session && g.own);
    const own = guest?.own;
    if (!guest || !own) return undefined;
    return {
      prompt: async (params) => {
        // A prompt took it over meanwhile: it was shown as that turn's.
        if (guest.run) return { stopReason: "end_turn" };
        const run: Run = {
          emit: params.onUpdate,
          prefix: `${guest.name}-main`,
          messages: new Map(),
          done: new AbortController(),
          titles: new Map(),
        };
        guest.run = run;
        own.run = run;
        try {
          for (const update of own.held.splice(0)) this.streamUpdate(run, update);
          if (own.stopReason === undefined) {
            await new Promise<void>((resolve) => {
              own.end = resolve;
            });
          }
          return { stopReason: own.stopReason ?? "end_turn" };
        } finally {
          clearTimeout(own.quiet);
          run.done.abort();
          if (guest.run === run) guest.run = undefined;
          if (guest.own === own) guest.own = undefined;
        }
      },
      cancel: () => {
        guest.conn.notify("session/cancel", { sessionId: guest.acpSession });
        this.endOwnTurn(own, "cancelled");
      },
    };
  }

  /**
   * A native subagent starting, running again or ending (both forms the adapters send: the
   * ACP RFD's subagent_update, Claude Code's; AIR's subagent_spawned and
   * subagent_state_update, Codex's). Returns whether the update was one of those.
   */
  private nativeLifecycle(guest: Guest, update: Record<string, unknown>): boolean {
    const kind = update.sessionUpdate;
    if (kind === "subagent_update") {
      const acpSession = str(update.sessionId);
      if (acpSession === undefined) return true;
      const child = this.childOf(guest, acpSession, false);
      const title = str(update.title);
      if (title !== undefined) child.name = title;
      const description = str(update.description);
      if (description !== undefined) child.description = description;
      if (isRecord(update.capabilities)) child.cancel = isRecord(update.capabilities.cancel);
      const state = isRecord(update.state) ? update.state : undefined;
      if (state?.state === "running" && !child.running) {
        child.id = `n_${crypto.randomUUID().slice(0, 8)}`;
        child.running = true;
        child.started = false;
      } else if (state?.state === "idle" || state?.state === "unknown") {
        const reason = str(state.stopReason);
        const error = isRecord(state.error) ? str(state.error.message) : undefined;
        let stop: StopReason = "end_turn";
        if (reason === "error" || state.state === "unknown") stop = "error";
        else if (reason !== undefined && STOP_REASONS.has(reason)) stop = reason as StopReason;
        const why = state.state === "unknown" ? "lost track of it" : undefined;
        this.endChild(guest, child, stop, error ?? why);
      }
      return true;
    }
    if (kind === "subagent_spawned") {
      const acpSession = str(update.subagentSessionId);
      if (acpSession === undefined) return true;
      const child = this.childOf(guest, acpSession, true);
      const name = str(update.name);
      if (name !== undefined) child.name = name;
      child.id = `n_${crypto.randomUUID().slice(0, 8)}`;
      child.running = true;
      child.started = false;
      this.startChild(guest, child, str(update.prompt) ?? str(update.task));
      return true;
    }
    if (kind === "subagent_state_update") {
      const acpSession = str(update.subagentSessionId);
      const child = acpSession === undefined ? undefined : guest.children.get(acpSession);
      if (!child) return true;
      const state = str(update.state);
      if (state === "completed") this.endChild(guest, child, "end_turn");
      else if (state === "cancelled") this.endChild(guest, child, "cancelled");
      else this.endChild(guest, child, "error", state === "failed" ? undefined : state);
      return true;
    }
    return false;
  }

  /** The native subagent that reports in this ACP session; a new one the first time. */
  private childOf(guest: Guest, acpSession: string, background: boolean): NativeChild {
    const child = guest.children.get(acpSession);
    if (child) return child;
    // Its label (and session) stays when it runs again, so its transcript goes on where it left off.
    const label = `${guest.name}/${crypto.randomUUID().slice(0, 8)}`;
    const made: NativeChild = {
      handle: acpSession,
      id: "",
      running: false,
      started: false,
      background,
      cancel: false,
      stream: {
        emit: (event) => this.options.append(made.session ?? guest.session, event),
        agent: label,
        prefix: label,
        messages: new Map(),
        titles: new Map(),
      },
    };
    guest.children.set(acpSession, made);
    return made;
  }

  /**
   * Logs the start of a native subagent's run, with its task: the prompt, else the adapter's
   * short description of it.
   */
  private startChild(guest: Guest, child: NativeChild, task?: string): void {
    child.started = true;
    let text = "";
    if (task !== undefined) text = task;
    else if (child.description !== undefined) text = child.description;
    else if (child.name !== undefined) text = child.name;
    // Its session, made the first time it runs; its events there are its own agent's.
    if (child.session === undefined && this.options.childSession) {
      child.session = this.options.childSession(guest.session, guest.name, child.handle);
      delete child.stream.agent;
    }
    const started: Extract<SessionEvent, { type: "subagent_started" }> = {
      type: "subagent_started",
      id: child.id,
      agent: child.stream.prefix,
      task: text,
    };
    started.name = child.name ?? this.title(guest.name);
    started.native = true;
    started.handle = child.handle;
    if (child.background) started.background = true;
    if (child.session !== undefined) started.child = child.session;
    this.options.append(guest.session, started);
    if (child.session !== undefined) {
      // Its questions show in the session that started it too, as a delegation's do.
      child.unmirror?.();
      child.unmirror = this.options.asker.mirror(child.session, guest.session);
      // What the user wrote in its session, passed on, is logged there already.
      if (!this.relays.delete(child.session)) {
        this.options.append(child.session, { type: "user_message", app: "delegate", text });
      }
    }
  }

  /**
   * What the user wrote in a native subagent's session went to the agent that started it, to
   * pass on (see daemon.ts): its run then starts with it, and is not logged a second time. A
   * turn of that agent that began after it and ends without it having passed it on says so.
   */
  relayed(child: string, parent: string): void {
    this.relays.set(child, { parent, at: Date.now() });
  }

  /** The turn of the session's agent that began at `began` ended: relays it left alone end. */
  private relaysLeft(guest: Guest, began: number): void {
    for (const [child, relay] of this.relays) {
      if (relay.parent !== guest.session || relay.at > began) continue;
      this.relays.delete(child);
      this.options.append(child, {
        type: "turn_ended",
        stopReason: "error",
        error: `${this.title(guest.name)} didn't pass it on`,
      });
    }
  }

  private endChild(guest: Guest, child: NativeChild, stopReason: StopReason, error?: string): void {
    if (!child.running) return;
    child.running = false;
    if (!child.started) this.startChild(guest, child);
    const ended: Extract<SessionEvent, { type: "subagent_ended" }> = {
      type: "subagent_ended",
      id: child.id,
      stopReason,
    };
    if (error !== undefined) ended.error = error;
    if (child.session !== undefined) {
      const turn: Extract<SessionEvent, { type: "turn_ended" }> = {
        type: "turn_ended",
        stopReason,
      };
      if (error !== undefined) turn.error = error;
      this.options.append(child.session, turn);
      child.unmirror?.();
      child.unmirror = undefined;
      this.options.asker.dropAll(child.session);
    }
    this.options.append(guest.session, ended);
  }

  /**
   * Whether the session's agent runs on `name` and offers the slash command `command` (its
   * adapter is running; see quietCommand).
   */
  offers(session: string, name: string, command: string): boolean {
    const guest = this.mainGuest(session, name);
    return guest?.commands?.some((c) => c.name === command) === true;
  }

  /**
   * Runs a slash command of the session's agent (on harness `name`) without logging anything,
   * for the daemon to read its answer: the text of its last message. Waits for a running turn,
   * as a prompt does. Rejects with BadRequestError when it can't.
   */
  async quietCommand(session: string, name: string, text: string): Promise<string> {
    const guest = this.mainGuest(session, name);
    if (!guest) throw new BadRequestError(`${this.title(name)} isn't running in this session`);
    const result = await this.enqueue(guest, text, { emit: () => {} });
    if (result.stopReason !== "end_turn") {
      throw new BadRequestError(result.error ?? `${text}: ${result.stopReason}`);
    }
    return result.text;
  }

  private mainGuest(session: string, name: string): Guest | undefined {
    return [...this.live].find((g) => g.main && g.session === session && g.name === name);
  }

  /** Stops the native subagent working in this session, when its adapter lets it be stopped. */
  cancelNativeSession(session: string): void {
    for (const guest of this.live) {
      for (const [acpSession, child] of guest.children) {
        if (child.session !== session || !child.running || !child.cancel) continue;
        guest.conn.notify("session/cancel", { sessionId: acpSession });
      }
    }
  }

  /** Whether a native subagent is running in this session (see GuestManagerOptions.childSession). */
  runsNative(session: string): boolean {
    for (const guest of this.live) {
      for (const child of guest.children.values()) {
        if (child.session === session && child.running) return true;
      }
    }
    return false;
  }

  /**
   * Stops a native subagent of the session's agent (its id is in subagent_started), when its
   * adapter lets it be stopped alone. False when there is no such subagent running.
   */
  cancelNative(session: string, id: string): boolean {
    for (const guest of this.live) {
      if (guest.session !== session) continue;
      for (const [acpSession, child] of guest.children) {
        if (child.id !== id || !child.running || !child.cancel) continue;
        guest.conn.notify("session/cancel", { sessionId: acpSession });
        return true;
      }
    }
    return false;
  }

  /** One ACP session's update into the log: what the agent says, its plan, its tool calls. */
  private streamUpdate(run: Stream, acpUpdate: Record<string, unknown>): void {
    const update = withNotebookEdit(acpUpdate);
    const { agent, prefix } = run;
    const kind = update.sessionUpdate;
    if (kind === "agent_message_chunk" || kind === "agent_thought_chunk") {
      const text = isRecord(update.content) ? str(update.content.text) : undefined;
      if (text === undefined || text === "") return;
      const given = str(update.messageId);
      if (given !== undefined) run.messageId = `${prefix}-${given}`;
      else if (run.messageKind !== kind || run.messageId === undefined) {
        run.messageId = `${prefix}-${crypto.randomUUID()}`;
      }
      run.messageKind = kind;
      if (kind === "agent_message_chunk") {
        run.messages.set(run.messageId, (run.messages.get(run.messageId) ?? "") + text);
      }
      run.emit(labelled({ type: kind, messageId: run.messageId, text }, agent));
      return;
    }
    // Not output of the agent: it neither ends nor splits the current message.
    if (kind === "usage_update") {
      const usage = usageOf(update, agent);
      if (usage) run.emit(usage);
      return;
    }
    // Anything else ends the current message.
    run.messageKind = undefined;
    if (kind === "plan") {
      if (Array.isArray(update.entries)) {
        run.emit(labelled({ type: "plan", entries: planEntries(update.entries) }, agent));
      }
      return;
    }
    const id = str(update.toolCallId);
    if (id === undefined) return;
    const toolCallId = `${prefix}:${id}`;
    const status = str(update.status);
    const listed = toContent(update.content, this.options.storeImage);
    const content = listed?.length ? listed : (mcpResultText(update.rawOutput) ?? listed);
    const rawTitle = str(update.title) ?? run.titles.get(toolCallId);
    if (rawTitle !== undefined) run.titles.set(toolCallId, rawTitle);
    if (kind === "tool_call") {
      const { title, tool } = describeCall(rawTitle ?? "tool", update.rawInput);
      const toolKind = str(update.kind);
      const call: ToolCall = {
        type: "tool_call",
        toolCallId,
        title,
        kind: toolKind !== undefined && KINDS.has(toolKind) ? (toolKind as ToolKind) : "other",
        status:
          status !== undefined && STATUSES.has(status) ? (status as ToolCallStatus) : "pending",
      };
      if (agent !== undefined) call.agent = agent;
      if (tool !== undefined) call.tool = tool;
      if (update.rawInput !== undefined) call.rawInput = update.rawInput;
      run.emit(call);
      if (content?.length) run.emit({ type: "tool_call_update", toolCallId, content });
    } else if (kind === "tool_call_update") {
      const out: ToolCallUpdate = { type: "tool_call_update", toolCallId };
      // Adapters stream a call's input after it started; the title can say more then.
      if (update.title !== undefined || update.rawInput !== undefined) {
        const { title, tool } = describeCall(rawTitle ?? "tool", update.rawInput);
        out.title = title;
        if (tool !== undefined) out.tool = tool;
      }
      if (update.rawInput !== undefined) out.rawInput = update.rawInput;
      if (status !== undefined && STATUSES.has(status)) out.status = status as ToolCallStatus;
      if (content !== undefined) out.content = content;
      if (update.rawOutput !== undefined) out.rawOutput = update.rawOutput;
      if (Object.keys(out).length > 2) run.emit(out);
    }
  }

  private async onRequest(
    guest: Guest | undefined,
    method: string,
    params: unknown,
  ): Promise<unknown> {
    if (method !== "session/request_permission" || !isRecord(params)) {
      throw new AcpError(`${method} is not supported`, -32601);
    }
    const options = this.permissionOptions(params.options);
    const cancelled = { outcome: { outcome: "cancelled" } };
    // A native subagent asks in its own session; one in the background may outlive the turn.
    const acpSession = str(params.sessionId);
    const child =
      acpSession === undefined || acpSession === guest?.acpSession
        ? undefined
        : guest?.children.get(acpSession);
    // (Work of the agent's own, before its turn started, may ask too.)
    const busy = guest?.run !== undefined || guest?.own !== undefined || child?.running === true;
    if (!guest || !busy || options.length === 0) return cancelled;
    const stream = child?.stream ?? {
      agent: guest.main ? undefined : guest.name,
      prefix: guest.main ? `${guest.name}-main` : guest.name,
    };
    const call: Record<string, unknown> = withNotebookEdit(Object.assign({}, params.toolCall));
    const content = toContent(call.content, this.options.storeImage);
    const callKind = str(call.kind);
    const kind: ToolKind | undefined =
      callKind !== undefined && KINDS.has(callKind) ? (callKind as ToolKind) : undefined;
    const isEdit = kind === "edit" || content?.some((c) => c.type === "diff") === true;
    // The same mapping for guests and the session's own agent.
    const config = this.options.configs[guest.name];
    const mode = this.options.approvals?.(guest.session);
    const allow =
      options.find((o) => o.kind === "allow_once") ??
      options.find((o) => o.kind.startsWith("allow"));
    // Sensitive files ask in every mode (a config of permissions: allow is the user's opt-out).
    // (Edits are not asked about by what they touch: their mode decides.)
    const touches: Touches = { kind: kind ?? "other", paths: [] };
    if (!isEdit && kind !== "delete" && kind !== "move") {
      touches.paths = callPaths(call);
      const command = callCommand(call);
      if (command !== undefined) touches.command = command;
    }
    // Neo's own tools: the gateway asks where it matters (delegate's gate), in every mode.
    if (allow && isNeoTool(call)) {
      return { outcome: { outcome: "selected", optionId: allow.id } };
    }
    // The agent starting a subagent of its own: as delegate, auto mode starts it without asking,
    // unless the user wants every start asked about (then nothing is allowed for good either).
    const spawn = isSubagentStart(call);
    const askAlways = spawn && this.options.alwaysAskForAgents?.() === true;
    if (spawn && allow && !askAlways && (config?.permissions === "allow" || mode === "auto")) {
      return { outcome: { outcome: "selected", optionId: allow.id } };
    }
    let sensitive: string[] = [];
    if (config?.permissions !== "allow" && !spawn) sensitive = this.guardOpen(guest, touches);
    if (sensitive.length === 0 && allow && !spawn) {
      const reads = kind === "read" || kind === "search";
      const edits = (mode === "edits" || mode === "review") && isEdit;
      if (config?.permissions === "allow" || mode === "auto" || reads || edits) {
        return { outcome: { outcome: "selected", optionId: allow.id } };
      }
    }
    const request: Omit<PermissionRequest, "id"> = {
      type: "permission_request",
      // A native subagent's by its label, which the session that started it knows it by.
      agent: child ? child.stream.prefix : (stream.agent ?? guest.name),
      title: describeCall(str(call.title) ?? "use a tool", call.rawInput).title,
      options,
    };
    if (sensitive.length > 0 && this.options.guard) {
      request.title = this.options.guard.title(guest.session, touches.kind, sensitive);
    }
    if (kind !== undefined) request.kind = kind;
    if (kind === "execute" && touches.command !== undefined) {
      // The command as code to read, under what it is for or a plain question, as neo asks.
      // (Claude Code sends the description as the content and the command as the title.)
      request.content = [{ type: "text", text: touches.command }];
      const rawDescription = isRecord(call.rawInput) ? str(call.rawInput.description) : undefined;
      const description = rawDescription?.replace(/\s+/g, " ").trim();
      if (sensitive.length === 0) request.title = description || RUN_TITLE;
    } else if (content?.length) request.content = content;
    if (spawn) {
      // Asked like delegate's start: what kind of subagent, then its task.
      const input: Record<string, unknown> = { ...(call.rawInput as object | undefined) };
      request.title = "Start a subagent?";
      const details = [{ key: "Agent", value: str(input.subagent_type) ?? "general-purpose" }];
      const what = str(input.description);
      if (what !== undefined) details.push({ key: "Task", value: what });
      if (input.run_in_background === true)
        details.push({ key: "Runs", value: "in the background" });
      request.details = details;
      const prompt = str(input.prompt);
      if (prompt !== undefined) request.content = [{ type: "text", text: prompt }];
      if (askAlways) request.options = options.filter((o) => o.kind !== "allow_always");
    }
    const toolCallId = str(call.toolCallId);
    if (toolCallId !== undefined) {
      request.toolCallId = `${stream.prefix}:${toolCallId}`;
    }
    // A native subagent's in its own session, copied to this one (see startChild).
    const { option: optionId, note } = await this.options.asker.askWithNote(
      child?.session ?? guest.session,
      request,
      guest.run?.done.signal,
    );
    if (optionId === undefined) return cancelled;
    const picked = options.find((o) => o.id === optionId);
    if (picked?.kind === "allow_always") this.options.guard?.remember(guest.session, sensitive);
    // Rejected without a note saying what to do instead: the work stops there, once the
    // adapter has the answer (a native subagent's alone; else the session's turn, or the guest's).
    if (picked?.kind.startsWith("reject") === true && note === undefined) {
      // Only the run that asked: one that ended meanwhile takes the next one along otherwise.
      const asked = child ? child.id : guest.run;
      setTimeout(() => {
        if (child ? !child.running || child.id !== asked : guest.run !== asked) return;
        if (guest.main && !child) this.options.stopTurn?.(guest.session);
        else guest.conn.notify("session/cancel", { sessionId: acpSession ?? guest.acpSession });
      }, 0);
    }
    if (note !== undefined) {
      // ACP's answer has no room for the note: it goes into the guest's running turn, once its
      // tool call has this answer; a guest that can't take that hears it with its next prompt.
      const said = { title: request.title, answer: picked?.name ?? optionId, note };
      if (guest.steering) setTimeout(() => void this.steerNote(guest, said), 0);
      else this.noteLater(guest, said);
    }
    return { outcome: { outcome: "selected", optionId } };
  }

  /** The note into the guest's running turn; for its next prompt when that doesn't work. */
  private async steerNote(guest: Guest, said: SaidNote): Promise<void> {
    if (guest.run !== undefined) {
      const text = `The user answered "${said.title}" with ${said.answer} and added: ${said.note}`;
      try {
        const reply = await guest.conn.request(STEER_METHOD, {
          sessionId: guest.acpSession,
          prompt: [{ type: "text", text }],
          _meta: { steering: { idleBehavior: "promptRequired" } },
        });
        if (isRecord(reply) && reply.outcome === "injected") return;
      } catch (err) {
        this.options.log.warn(`${guest.name}: passing a note failed: ${errorText(err)}`);
      }
    }
    this.noteLater(guest, said);
  }

  /** The note as a user_note: the agent hears it with its next prompt. */
  private noteLater(guest: Guest, said: SaidNote): void {
    const about = `${said.title} → ${said.answer}`;
    this.options.append(guest.session, { type: "user_note", note: said.note, about });
  }

  private guardOpen(guest: Guest, touches: Touches): string[] {
    return this.options.guard?.open(guest.session, touches) ?? [];
  }

  private permissionOptions(value: unknown): PermissionOption[] {
    if (!Array.isArray(value)) return [];
    const out: PermissionOption[] = [];
    for (const o of value) {
      if (!isRecord(o)) continue;
      const id = str(o.optionId);
      const kind = str(o.kind);
      if (id === undefined || kind === undefined) continue;
      if (!["allow_once", "allow_always", "reject_once", "reject_always"].includes(kind)) continue;
      out.push({ id, name: str(o.name) ?? id, kind: kind as PermissionOption["kind"] });
    }
    return out;
  }
}

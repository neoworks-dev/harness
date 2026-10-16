// The ACP-style contract between the daemon and an agent. The daemon only talks to agents through
// this, so the pi harness can be swapped for another implementation (or a guest bridge) later.
//
// Flow per session: newSession/loadSession once, then prompt() per turn. The daemon appends every
// AgentUpdate to the session log in the order onUpdate is called, then appends turn_ended with
// the returned stop reason. The daemon writes user_message itself before calling prompt().
import type {
  AgentMessageChunk,
  AgentThoughtChunk,
  AppConnected,
  AppDisconnected,
  EditsReverted,
  FocusChanged,
  GuestTurn,
  HarnessMessage,
  Plan,
  SessionEvent,
  StopReason,
  ToolCall,
  ToolCallContent,
  ToolCallUpdate,
  ToolKind,
  Usage,
  UserNote,
} from "@suite/protocol/types";

/** What an agent may emit during a prompt (ACP session/update plus harness_message and usage). */
export type AgentUpdate =
  AgentMessageChunk | AgentThoughtChunk | ToolCall | ToolCallUpdate | Plan | Usage | HarnessMessage;

/** Connector events (and what guests did on their own) that get prepended to the next user message. */
export type ContextEvent =
  AppConnected | AppDisconnected | FocusChanged | GuestTurn | EditsReverted | UserNote;

/** An MCP server for the agent (ACP's McpServerHttp shape). */
export interface McpServer {
  name: string;
  url: string;
  headers: { name: string; value: string }[];
}

/** ACP's config option categories the suite lets the user set (/model, /effort, /mode). */
export type ConfigCategory = "model" | "thought_level" | "mode";

/** One settable option of a session, as ACP's session config options describe it. */
export interface ConfigOption {
  category: ConfigCategory;
  /** The current value, if known. */
  current?: string;
  /** The values to offer. A harness may accept others (pi: any provider/model). */
  values: { value: string; name: string; description?: string }[];
}

/** What an agent's own tool asks the user before it acts. */
export interface PermissionAsk {
  /** What it wants to do: the file to write, the command to run. */
  title: string;
  kind: ToolKind;
  toolCallId?: string;
  /** What to preview; for an edit, the diff that has not been written yet. */
  content?: ToolCallContent[];
  /** The file a read is about (absolute): sensitive ones ask in every approvals mode. */
  path?: string;
  /**
   * "before" (the default): asked before acting. "after": an edit was written and the user may
   * take it back (review mode; only asked when the "before" answer had `review`); `content` has
   * its diff, and a refusal reverts it.
   */
  phase?: "before" | "after";
}

/** The user's answer to a PermissionAsk. */
export interface PermissionAnswer {
  allowed: boolean;
  /** What the user added for the agent ("use tabs instead"); the tool puts it in its result. */
  note?: string;
  /** "before" answers: go ahead, then ask again with phase "after" once the edit is written. */
  review?: boolean;
  /** "after" answers: whether the refused edit was put back (false: the file changed since). */
  reverted?: boolean;
}

/**
 * Asks whether to go ahead. The signal drops the question when the turn is cancelled (the
 * answer is then not allowed).
 */
export type RequestPermission = (
  request: PermissionAsk,
  signal?: AbortSignal,
) => Promise<PermissionAnswer>;

export interface NewSessionParams {
  sessionId: string;
  /** Working directory for the core tools. */
  cwd: string;
  /** The daemon's MCP gateway; the agent reaches connector tools through it. */
  mcpServers: McpServer[];
  /** Resolves an asset://… handle to its file path (undefined: unknown handle). */
  resolveAsset?: (handle: string) => string | undefined;
  /** What the user chose for this session; unset categories keep the harness's default. */
  settings?: Partial<Record<ConfigCategory, string>>;
  /**
   * Asked before the agent's own bash, write, edit and read act (the gateway tools never ask),
   * and after an edit when the user reviews each one. Absent: they just run.
   */
  requestPermission?: RequestPermission;
}

export interface LoadSessionParams extends NewSessionParams {
  /**
   * Every event in the session log so far, in seq order. The agent rebuilds its context from
   * the events it wrote itself (harness_message) and ignores the rest. The log may end in the
   * middle of a turn (daemon killed); the agent must still be able to continue.
   */
  history: SessionEvent[];
}

export interface PromptParams {
  /** User text as typed (no tags). */
  text: string;
  /** App the input came from. */
  app: string;
  /** Absolute paths of attached images. */
  images?: string[];
  /** Connect/disconnect/focus events since the previous prompt, oldest first. May be empty. */
  pendingEvents: ContextEvent[];
  /**
   * What was said in turns another harness ran since this one last did (a <conversation-so-far>
   * block), put before the pending events. Absent when there is nothing to catch up on.
   */
  summary?: string;
  /** Called synchronously for each update, in order. */
  onUpdate: (update: AgentUpdate) => void;
}

export interface PromptResult {
  stopReason: StopReason;
  /** Set when stopReason is "error". */
  error?: string;
}

export interface AgentSession {
  /** Runs one turn. Never rejects: failures resolve with stopReason "error". */
  prompt(params: PromptParams): Promise<PromptResult>;
  /** Cancels the running prompt, which then resolves with stopReason "cancelled". No-op when idle. */
  cancel(): void;
  /** The options the user can set; absent when the harness has none. */
  configOptions?(): ConfigOption[];
  /** Sets one for the following turns. Rejects with a message the user can read. */
  setConfigOption?(category: ConfigCategory, value: string): Promise<void>;
}

export interface Agent {
  /** Short name, e.g. "pi"; also used as HarnessMessage.harness. */
  readonly name: string;
  newSession(params: NewSessionParams): Promise<AgentSession>;
  loadSession(params: LoadSessionParams): Promise<AgentSession>;
}

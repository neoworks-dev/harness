// The ACP-style contract between the daemon and an agent. The daemon only talks to agents through
// this, so the neo harness can be swapped for another implementation (or a guest bridge) later.
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
  Notice,
  Plan,
  SessionEvent,
  ShowDontTell,
  StopReason,
  ToolCall,
  ToolCallContent,
  ToolCallUpdate,
  ToolKind,
  Usage,
  UserNote,
  Work,
} from "@neoworks-dev/neo-protocol/types";
import type { SafeSearch } from "@neoworks/websearch";
import type { AgentInfo } from "./agents.ts";

export type { SafeSearch };
export type { AgentInfo } from "./agents.ts";

/**
 * What an agent may emit during a prompt (ACP session/update plus harness_message and usage, and
 * a notice: a line for the user that the agent does not see, e.g. a hook that failed).
 */
export type AgentUpdate =
  | AgentMessageChunk
  | AgentThoughtChunk
  | ToolCall
  | ToolCallUpdate
  | Plan
  | Usage
  | HarnessMessage
  | Notice;

/**
 * Connector events (and what guests did on their own, the user's notes and settings that change
 * how the agent answers) that get prepended to the next user message.
 */
export type ContextEvent =
  | AppConnected
  | AppDisconnected
  | FocusChanged
  | GuestTurn
  | EditsReverted
  | UserNote
  | ShowDontTell;

/** An MCP server for the agent (ACP's McpServerHttp shape). */
export interface McpServer {
  name: string;
  url: string;
  headers: { name: string; value: string }[];
}

/** ACP's config option categories neo lets the user set (/model, /effort, /mode). */
export type ConfigCategory = "model" | "thought_level" | "mode";

/** One settable option of a session, as ACP's session config options describe it. */
export interface ConfigOption {
  category: ConfigCategory;
  /** The current value, if known. */
  current?: string;
  /** The values to offer. A harness may accept others (neo: any provider/model). */
  values: { value: string; name: string; description?: string }[];
}

/** What a harness wants to ask or tell the user after a setting was set (see followUp). */
export interface FollowUp {
  /** A second question. Picking a value sets the same category again; no pick: nothing happens. */
  choice?: {
    title: string;
    options: { value: string; name: string; description?: string }[];
  };
  /** Something to tell the user (a lookup failed and the step was skipped). */
  notice?: string;
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
  /**
   * What a PreToolUse hook (hooks.ts) decided: "allow" replaces the question the approvals mode
   * would ask (a sensitive file still asks), "ask" asks even where the mode would not. Absent: the
   * approvals mode decides.
   */
  hook?: "allow" | "ask";
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
  /**
   * A command the user sent to the background (see AgentSession.background) ended: `text` says
   * how, for the agent to hear of it. Absent: nobody is told.
   */
  backgroundEnded?: (text: string) => void;
  /**
   * The user's safe-search choice (/safesearch), read at each web search; undefined: the
   * harness's configured default.
   */
  safeSearch?: () => SafeSearch | undefined;
  /** The user's own words for the system prompt (/system-prompt), after the rest of it. */
  promptAddition?: string;
  /**
   * What the user does with neo: for work on the computer (not coding) there is no shell.
   * Absent: coding, every tool.
   */
  work?: Work;
  /**
   * Set when this session is a subagent of another (delegate): its hooks run as SubagentStop, not
   * Stop. `definition` names the agent definition (agents.ts) it runs as, if any.
   */
  subagent?: { definition?: string };
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
  /**
   * Takes the messages other agents sent this one since the last call (already formatted as
   * <message from=…>…</message>). neo calls it when a turn starts and after each batch of tool
   * calls and puts them in the conversation as user messages; an adapter that can't be told
   * anything mid-turn calls it when the turn starts (the daemon adds the rest to its tool
   * results, see messages.ts).
   */
  takeMessages?: () => string[];
  /**
   * Takes what the user sent while the turn runs (the daemon has logged each as user_message by
   * then). neo calls it where it calls takeMessages and puts them in the conversation as the
   * user's messages; an adapter that can't be told anything mid-turn doesn't call it, and they
   * start the next turn instead.
   */
  takeInputs?: () => { text: string; images?: string[] }[];
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
  /**
   * Hands what the user sent to the running turn at once (ACP steering). Resolves true when the
   * turn took it, false when there is no turn to take it (it then waits for a turn of its own).
   * Absent when the harness can't be told anything mid-turn, or asks for it itself (neo:
   * PromptParams.takeInputs).
   */
  steer?(input: { text: string; images?: string[] }): Promise<boolean>;
  /**
   * Sends a running command (a bash call) to the background: the call ends at once with where
   * its output goes, the command runs on. Resolves false when no such call runs.
   */
  background?(toolCallId: string): Promise<boolean>;
  /** The options the user can set; absent when the harness has none. */
  configOptions?(): ConfigOption[];
  /** Like configOptions, for a harness that has to look something up first; preferred when set. */
  loadConfigOptions?(): Promise<ConfigOption[]>;
  /** Sets one for the following turns. Rejects with a message the user can read. */
  setConfigOption?(category: ConfigCategory, value: string): Promise<void>;
  /**
   * Asked by the daemon after a successful setConfigOption the user made with /model, /effort or
   * /mode: a second step the harness knows about (neo: which OpenRouter host runs the model).
   * Never rejects; undefined: nothing more to ask.
   */
  followUp?(category: ConfigCategory, value: string): Promise<FollowUp | undefined>;
  /** The session is over (the daemon stops): runs the SessionEnd hooks. Never rejects. */
  close?(reason: string): Promise<void>;
}

/** What generateText takes. */
export interface GenerateTextParams {
  /** The model to ask: a value of the harness's model setting; absent: the harness's own default. */
  model?: string;
  system: string;
  prompt: string;
  signal?: AbortSignal;
}

export interface Agent {
  /** Short name, e.g. "neo"; also used as HarnessMessage.harness. */
  readonly name: string;
  newSession(params: NewSessionParams): Promise<AgentSession>;
  loadSession(params: LoadSessionParams): Promise<AgentSession>;
  /**
   * One model call outside any session, with no tools and nothing logged, for short generated text
   * (a session's title). Resolves with the model's reply; rejects with a message the user can read.
   * Absent when the harness can't do it.
   */
  generateText?(params: GenerateTextParams): Promise<string>;
  /**
   * Every agent definition (agents.ts) a session in `cwd` has, the winner of a name first:
   * delegate can hand a task to one by name, and the harness runs it as a subagent (NewSessionParams.subagent.definition). Absent
   * when the harness has none.
   */
  agents?(cwd: string): AgentInfo[];
}

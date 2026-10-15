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
  FocusChanged,
  HarnessMessage,
  SessionEvent,
  StopReason,
  ToolCall,
  ToolCallUpdate,
} from "@suite/protocol/types";

/** What an agent may emit during a prompt (ACP session/update plus harness_message). */
export type AgentUpdate =
  AgentMessageChunk | AgentThoughtChunk | ToolCall | ToolCallUpdate | HarnessMessage;

/** Connector events that get prepended to the next user message. */
export type ContextEvent = AppConnected | AppDisconnected | FocusChanged;

export interface NewSessionParams {
  sessionId: string;
  /** Working directory for the core tools. */
  cwd: string;
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
  /** Connect/disconnect/focus events since the previous prompt, oldest first. May be empty. */
  pendingEvents: ContextEvent[];
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
}

export interface Agent {
  /** Short name, e.g. "pi"; also used as HarnessMessage.harness. */
  readonly name: string;
  newSession(params: NewSessionParams): Promise<AgentSession>;
  loadSession(params: LoadSessionParams): Promise<AgentSession>;
}

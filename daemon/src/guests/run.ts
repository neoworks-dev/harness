// What running a task on a subagent takes and gives back (ACP guests and neo alike).
import type { AgentUpdate, ContextEvent, PromptResult } from "@neoworks-dev/neo/agent";
import type { HarnessSettings } from "../settings.ts";

export interface GuestRunResult extends PromptResult {
  /** The subagent's last message. */
  text: string;
}

export interface RunOptions {
  /** Where its updates go as they come. */
  emit: (update: AgentUpdate) => void;
  signal?: AbortSignal;
  images?: string[];
  /** The session's pending events: its guest turns and reverted edits reach the harness. */
  events?: ContextEvent[];
  /** Turns another harness ran that this one missed (a <conversation-so-far> block). */
  summary?: string;
  /**
   * Settings for this one task (delegate's model and effort), applied over the user's and put
   * back afterwards. They are not recorded as the user's choices.
   */
  settings?: HarnessSettings;
  /**
   * Takes the messages other agents sent this one (see messages.ts): neo reads them as its turn
   * goes, an adapter when the turn starts (the gateway adds later ones to its tool results).
   */
  takeMessages?: () => string[];
  /**
   * neo only: run as this agent definition (harness agents.ts), a conversation of its own next to
   * the plain neo subagent's.
   */
  definition?: string;
}

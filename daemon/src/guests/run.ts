// What running a task on a subagent takes and gives back (ACP guests and neo alike).
import type { AgentUpdate, ContextEvent, PromptResult } from "@suite/harness/agent";
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
}

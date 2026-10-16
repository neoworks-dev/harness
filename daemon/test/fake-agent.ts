// A controllable Agent for daemon tests: every prompt blocks until the test releases it.
import type {
  Agent,
  AgentSession,
  LoadSessionParams,
  NewSessionParams,
  PromptParams,
  PromptResult,
} from "@suite/harness/agent";

interface Pending {
  params: PromptParams;
  resolve: (result: PromptResult) => void;
}

export class FakeAgent implements Agent {
  constructor(readonly name = "fake") {}

  readonly created: NewSessionParams[] = [];
  readonly loaded: LoadSessionParams[] = [];
  readonly prompts: PromptParams[] = [];
  active = 0;
  maxActive = 0;
  private waiting: Pending[] = [];
  private waiters: (() => void)[] = [];

  newSession(params: NewSessionParams): Promise<AgentSession> {
    this.created.push(params);
    return Promise.resolve(this.session());
  }

  loadSession(params: LoadSessionParams): Promise<AgentSession> {
    this.loaded.push(params);
    return Promise.resolve(this.session());
  }

  /** Resolves once `n` prompts in total have started. */
  async started(n: number): Promise<void> {
    while (this.prompts.length < n) {
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
  }

  /** Finishes the oldest running prompt. */
  release(result: PromptResult = { stopReason: "end_turn" }): void {
    const next = this.waiting.shift();
    if (!next) throw new Error("no prompt is running");
    next.resolve(result);
  }

  private session(): AgentSession {
    let current: Pending | undefined;
    return {
      prompt: (params) => {
        this.prompts.push(params);
        this.active += 1;
        this.maxActive = Math.max(this.maxActive, this.active);
        params.onUpdate({ type: "agent_message_chunk", messageId: "m", text: params.text });
        for (const wake of this.waiters.splice(0)) wake();
        return new Promise<PromptResult>((resolve) => {
          current = {
            params,
            resolve: (result) => {
              this.active -= 1;
              this.waiting = this.waiting.filter((p) => p !== current);
              resolve(result);
            },
          };
          this.waiting.push(current);
        });
      },
      cancel: () => current?.resolve({ stopReason: "cancelled" }),
    };
  }
}

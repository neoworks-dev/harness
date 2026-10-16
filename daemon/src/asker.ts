// Questions for the user, answered with buttons in any pane: permission requests (a guest wants
// to run a tool, the agent wants to start a guest) and choices (a model, a conversation to
// resume). The question goes into the session log; the answer (action suite.permit or
// suite.choose) settles it and is logged too.
import type { SessionEvent } from "@suite/protocol/types";
import { BadRequestError } from "./focus.ts";

export type PermissionRequest = Extract<SessionEvent, { type: "permission_request" }>;
export type Choice = Extract<SessionEvent, { type: "choice" }>;

interface Pending {
  session: string;
  kind: "permission" | "choice";
  options: string[];
  resolve: (option: string | undefined) => void;
}

export class Asker {
  private readonly pending = new Map<string, Pending>();

  constructor(private readonly append: (session: string, event: SessionEvent) => void) {}

  /**
   * Asks and waits. Resolves with the picked option id, or undefined when the signal aborts
   * first (the answer is then logged as none).
   */
  ask(
    session: string,
    question: Omit<PermissionRequest, "id"> | Omit<Choice, "id">,
    signal?: AbortSignal,
  ): Promise<string | undefined> {
    if (signal?.aborted) return Promise.resolve(undefined);
    const id = `${question.type === "choice" ? "c" : "p"}_${crypto.randomUUID().slice(0, 8)}`;
    return new Promise((resolve) => {
      const onAbort = (): void => this.settle(id, undefined);
      signal?.addEventListener("abort", onAbort, { once: true });
      this.pending.set(id, {
        session,
        kind: question.type === "choice" ? "choice" : "permission",
        options: question.options.map((o) => o.id),
        resolve: (option) => {
          signal?.removeEventListener("abort", onAbort);
          resolve(option);
        },
      });
      this.append(session, { ...question, id } as SessionEvent);
    });
  }

  /** A button press: suite.permit or suite.choose with {id, option}. */
  answer(session: string, kind: Pending["kind"], id: string, option: string, by: string): void {
    const pending = this.pending.get(id);
    if (!pending || pending.session !== session || pending.kind !== kind) {
      throw new BadRequestError(`${id} was already answered or isn't open`);
    }
    if (!pending.options.includes(option))
      throw new BadRequestError(`${id} has no option ${option}`);
    this.settle(id, option, by);
  }

  /** Drops every open question of a session (or all). */
  dropAll(session?: string): void {
    for (const [id, pending] of this.pending) {
      if (session === undefined || pending.session === session) this.settle(id, undefined);
    }
  }

  private settle(id: string, option: string | undefined, by?: string): void {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    const event: SessionEvent =
      pending.kind === "choice" ? { type: "choice_made", id } : { type: "permission_resolved", id };
    if (option !== undefined) event.option = option;
    if (by !== undefined) event.by = by;
    this.append(pending.session, event);
    pending.resolve(option);
  }
}

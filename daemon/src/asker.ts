// Questions for the user, answered with buttons in any pane: permission requests (a guest wants
// to run a tool, the agent wants to start a guest) and choices (a model, a conversation to
// resume). The question goes into the session log; the answer (action neo.permit or
// neo.choose, with an optional note for the agent) settles it and is logged too.
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { BadRequestError } from "./focus.ts";

export type PermissionRequest = Extract<SessionEvent, { type: "permission_request" }>;
export type Choice = Extract<SessionEvent, { type: "choice" }>;

interface Pending {
  session: string;
  /** Sessions the question was copied to (see mirror): answering there settles it too. */
  mirrors: string[];
  kind: "permission" | "choice";
  options: string[];
  resolve: (answer: Answer) => void;
}

/** What the user picked (none: dropped before anyone answered) and what they added. */
export interface Answer {
  option?: string;
  note?: string;
}

export class Asker {
  private readonly pending = new Map<string, Pending>();
  /** Session → sessions its questions are copied to, so a parent shows a child's cards. */
  private readonly mirrored = new Map<string, Set<string>>();

  constructor(private readonly append: (session: string, event: SessionEvent) => void) {}

  /**
   * Asks and waits. Resolves with the picked option id, or undefined when the signal aborts
   * first (the answer is then logged as none). A note the user added becomes a user_note
   * event: the agent hears it with its next prompt. Callers that can hand the note to the
   * agent right away use askWithNote.
   */
  async ask(
    session: string,
    question: Omit<PermissionRequest, "id"> | Omit<Choice, "id">,
    signal?: AbortSignal,
  ): Promise<string | undefined> {
    const { option, note } = await this.askWithNote(session, question, signal);
    if (option !== undefined && note !== undefined) {
      const name = question.options.find((o) => o.id === option)?.name ?? option;
      this.append(session, { type: "user_note", note, about: `${question.title} → ${name}` });
    }
    return option;
  }

  /**
   * Copies the questions asked in `from` to `to` (and their answers) until the returned function
   * is called, so the user sees a delegated session's cards in the session that delegated.
   */
  mirror(from: string, to: string): () => void {
    const targets = this.mirrored.get(from) ?? new Set<string>();
    targets.add(to);
    this.mirrored.set(from, targets);
    return () => {
      targets.delete(to);
      if (targets.size === 0 && this.mirrored.get(from) === targets) this.mirrored.delete(from);
    };
  }

  /** Like ask, but the note comes back to the caller, who has to pass it on. */
  askWithNote(
    session: string,
    question: Omit<PermissionRequest, "id"> | Omit<Choice, "id">,
    signal?: AbortSignal,
  ): Promise<Answer> {
    if (signal?.aborted) return Promise.resolve({});
    const id = `${question.type === "choice" ? "c" : "p"}_${crypto.randomUUID().slice(0, 8)}`;
    return new Promise((resolve) => {
      const onAbort = (): void => this.settle(id, undefined);
      signal?.addEventListener("abort", onAbort, { once: true });
      const mirrors = [...(this.mirrored.get(session) ?? [])];
      this.pending.set(id, {
        session,
        mirrors,
        kind: question.type === "choice" ? "choice" : "permission",
        options: question.options.map((o) => o.id),
        resolve: (answer) => {
          signal?.removeEventListener("abort", onAbort);
          resolve(answer);
        },
      });
      for (const target of [session, ...mirrors]) {
        this.append(target, { ...question, id } as SessionEvent);
      }
    });
  }

  /** A button press: neo.permit or neo.choose with {id, option}. */
  answer(
    session: string,
    kind: Pending["kind"],
    id: string,
    option: string,
    by: string,
    note?: string,
  ): void {
    const pending = this.pending.get(id);
    if (
      !pending ||
      (pending.session !== session && !pending.mirrors.includes(session)) ||
      pending.kind !== kind
    ) {
      throw new BadRequestError(`${id} was already answered or isn't open`);
    }
    if (!pending.options.includes(option))
      throw new BadRequestError(`${id} has no option ${option}`);
    this.settle(id, option, by, note);
  }

  /** Drops every open question of a session (or all). */
  dropAll(session?: string): void {
    for (const [id, pending] of this.pending) {
      if (session === undefined || pending.session === session) this.settle(id, undefined);
    }
  }

  /**
   * Drops the open questions of a session that was deleted, unanswered and without logging:
   * its log is gone.
   */
  forget(session: string): void {
    for (const [id, pending] of this.pending) {
      if (pending.session !== session) continue;
      this.pending.delete(id);
      pending.resolve({});
    }
    this.mirrored.delete(session);
  }

  private settle(id: string, option: string | undefined, by?: string, note?: string): void {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    const event: SessionEvent =
      pending.kind === "choice" ? { type: "choice_made", id } : { type: "permission_resolved", id };
    if (option !== undefined) event.option = option;
    if (note !== undefined) event.note = note;
    if (by !== undefined) event.by = by;
    for (const target of [pending.session, ...pending.mirrors]) this.append(target, event);
    const answer: Answer = {};
    if (option !== undefined) answer.option = option;
    if (note !== undefined) answer.note = note;
    pending.resolve(answer);
  }
}

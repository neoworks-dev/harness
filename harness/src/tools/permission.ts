import type { PermissionAsk, RequestPermission } from "../agent.ts";

/** What the user's answer asks of the tool beyond going ahead. */
export interface Proceed {
  /** What the user added; goes into the tool's result. */
  note?: string;
  /** Review mode: ask again once the edit is written (see reviewEdit). */
  review: boolean;
}

/** The text with the user's note under it, when they gave one. */
export function withNote(text: string, note: string | undefined): string {
  return note === undefined ? text : `${text}\n\nThe user added: ${note}`;
}

/**
 * Asks before a tool acts, when the embedder gave a hook. A refusal (or a cancelled turn while
 * waiting) throws: neo turns a thrown error into the tool's error result, which the model reads
 * (with the user's note, if any).
 */
export async function askToProceed(
  ask: RequestPermission | undefined,
  request: PermissionAsk,
  declined: string,
  signal?: AbortSignal,
): Promise<Proceed> {
  if (!ask) return { review: false };
  const answer = await ask(request, signal);
  if (signal?.aborted) throw new Error("Cancelled before the user answered.");
  if (!answer.allowed) throw new Error(withNote(declined, answer.note));
  const proceed: Proceed = { review: answer.review === true };
  if (answer.note !== undefined) proceed.note = answer.note;
  return proceed;
}

/**
 * Review mode: the edit is written and the user may take it back. Returns the note to put in
 * the result; throws, telling the model, when they reverted it. A cancelled turn leaves the
 * edit as it is.
 */
export async function reviewEdit(
  ask: RequestPermission | undefined,
  proceed: Proceed,
  request: PermissionAsk,
  signal?: AbortSignal,
): Promise<string | undefined> {
  if (!ask || !proceed.review) return proceed.note;
  const answer = await ask({ ...request, phase: "after" }, signal);
  if (signal?.aborted) return proceed.note;
  if (!answer.allowed) {
    const said =
      answer.reverted === true
        ? "The user reverted this edit; the file is back as it was before."
        : "The user reverted this edit, but the file had changed since, so it was left as it is.";
    throw new Error(withNote(said, answer.note));
  }
  return answer.note ?? proceed.note;
}

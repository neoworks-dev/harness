// The logic of a secret question, apart from the window: which request is shown, which are
// answered as cancelled, and what is sent. Pure, so tests reach it (see the README on runes).
import type { SecretRequest } from "@neoworks-dev/neo-protocol/types";

/** The question on screen. It holds no answer: the typed text lives in the field only. */
export interface Ask {
  id: string;
  /** The session it was asked in. */
  session: string;
  title: string;
  placeholder?: string;
}

/** What to do with a request that just arrived. */
export interface Incoming {
  /** The question to show now (the new one, or the open one when nothing can show the new one). */
  show: Ask | undefined;
  /** Questions to answer as cancelled: the one it replaces, or itself when it cannot be shown. */
  cancel: string[];
}

/**
 * A request for the shown session replaces the open question, which is answered as cancelled.
 * A request for another session cannot be shown, and is answered as cancelled at once.
 */
export function incoming(
  open: Ask | undefined,
  request: SecretRequest,
  shownSession: string | undefined,
): Incoming {
  if (request.session !== shownSession) return { show: open, cancel: [request.id] };
  const ask: Ask = { id: request.id, session: request.session, title: request.title };
  if (request.placeholder !== undefined) ask.placeholder = request.placeholder;
  const cancel: string[] = [];
  if (open) cancel.push(open.id);
  return { show: ask, cancel };
}

/** What Enter sends: the text without surrounding blanks, or nothing while the field is empty. */
export function submitted(text: string): string | undefined {
  const value = text.trim();
  return value === "" ? undefined : value;
}

/** Why the open question must go, if it must: the daemon is gone, or the window left its session. */
export function stale(
  open: Ask,
  shownSession: string | undefined,
  connection: string,
): "session" | "connection" | undefined {
  if (connection !== "open") return "connection";
  if (open.session !== shownSession) return "session";
  return undefined;
}

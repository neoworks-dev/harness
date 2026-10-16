// Switching harness mid-session (/harness): the one that runs next never saw the turns the others
// ran, so its prompt starts with a short summary of them (what the user said and what the agent
// finally answered, no tool calls). A harness that keeps its own history is told only about the
// turns since its last one; a new conversation (first time on this harness, or its stored one
// is gone) gets all of them.
import type { SessionEventRecord } from "@suite/protocol/types";
import type { SessionStore } from "./store.ts";

/** Longest summary, in characters; older turns are dropped first. */
const MAX_CHARS = 6000;
/** Longest user message or reply in the summary. */
const MAX_TEXT = 1500;

export interface Turn {
  /** Seq of the turn's user_message. */
  seq: number;
  /** The harness that ran it. */
  harness: string;
  user: string;
  /** The agent's last message (not a guest's). */
  reply: string;
}

/**
 * The ended turns in the log, oldest first. A turn runs on the session's harness at its
 * user_message (the last /harness before it, else `main`). Turns the user gave a guest directly
 * (/claude task) are left out: the agent hears of those as guest_turn events.
 */
export function turnsOf(records: SessionEventRecord[], main: string): Turn[] {
  const turns: Turn[] = [];
  let harness = main;
  let open: { turn: Turn; messages: Map<string, string>; guest: boolean } | undefined;
  for (const { seq, event } of records) {
    if (event.type === "setting_changed") {
      if (event.option === "harness") harness = event.value;
    } else if (event.type === "user_message") {
      const turn: Turn = { seq, harness, user: event.text, reply: "" };
      open = { turn, messages: new Map(), guest: false };
    } else if (!open) {
      continue;
    } else if (event.type === "guest_turn") {
      open.guest = true;
    } else if (event.type === "agent_message_chunk" && event.agent === undefined) {
      open.messages.set(event.messageId, (open.messages.get(event.messageId) ?? "") + event.text);
    } else if (event.type === "turn_ended") {
      open.turn.reply = [...open.messages.values()].at(-1)?.trim() ?? "";
      if (!open.guest) turns.push(open.turn);
      open = undefined;
    }
  }
  return turns;
}

/** The turns `harness` doesn't know: the ones after its last own, or all of them. */
export function missedTurns(turns: Turn[], harness: string, all: boolean): Turn[] {
  if (all) return turns;
  return turns.slice(turns.findLastIndex((turn) => turn.harness === harness) + 1);
}

function cut(text: string): string {
  return text.length > MAX_TEXT ? `${text.slice(0, MAX_TEXT)}…` : text;
}

/** The <conversation-so-far> block for these turns; the newest are kept when it gets long. */
export function formatSummary(turns: Turn[]): string | undefined {
  if (turns.length === 0) return undefined;
  const entries = turns.map(
    (turn) =>
      `User: ${cut(turn.user)}\nAgent: ${turn.reply === "" ? "(no reply)" : cut(turn.reply)}`,
  );
  let size = 0;
  let first = entries.length;
  while (first > 0 && size + (entries[first - 1]?.length ?? 0) <= MAX_CHARS) {
    first--;
    size += (entries[first]?.length ?? 0) + 1;
  }
  // Even the newest turn fits: the texts are cut and MAX_CHARS is above two of them.
  const lines = ["Earlier turns of this conversation, run by another agent (tool calls left out):"];
  if (first > 0) lines.push(`(${first} older ${first === 1 ? "turn" : "turns"} left out)`);
  lines.push(...entries.slice(first));
  return `<conversation-so-far>\n${lines.join("\n")}\n</conversation-so-far>`;
}

/**
 * The summary for `harness`'s next prompt, or undefined when it missed nothing. Reads the full
 * log (with the replies) only when there is something to tell.
 */
export function handoffSummary(
  store: Pick<SessionStore, "read" | "readTypes">,
  session: string,
  harness: string,
  main: string,
  all = false,
): string | undefined {
  const types = ["setting_changed", "user_message", "guest_turn", "turn_ended"] as const;
  const missed = missedTurns(turnsOf(store.readTypes(session, [...types]), main), harness, all);
  const first = missed[0];
  if (!first) return undefined;
  const replies = new Map(
    turnsOf(store.read(session, first.seq), main).map((t) => [t.seq, t.reply]),
  );
  return formatSummary(missed.map((turn) => ({ ...turn, reply: replies.get(turn.seq) ?? "" })));
}

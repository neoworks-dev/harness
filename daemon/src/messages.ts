// Messages between the agents of a session: the session's own agent (`main`) and the subagents
// it delegated to (their ids, from subagent_started). The send_message and wait core tools land
// here, and so do background subagents' final replies.
//
// Every message is logged as an agent_message event (the chat and the subagents' transcripts draw
// it) and queued in the recipient's inbox, in memory: subagents don't outlive the daemon, and a
// message nobody read when the daemon stopped goes with it. Whoever runs the recipient takes the
// inbox, which is how a message is delivered, once:
//   - neo (the session's agent or a subagent) takes it after each tool batch, as user messages in
//     its conversation (PromptParams.takeMessages);
//   - an ACP agent (Claude Code, Codex) cannot be told anything mid-turn, but every neo tool it
//     calls goes through the gateway, which appends the inbox to that tool's result; what is left
//     when its turn ends starts its next turn;
//   - `wait` takes it too, and returns it.
// A message for an idle `main` starts a turn (`wake`); a subagent is never idle, it ends.
//
// Limits keep agents from talking forever: PAIR_LIMIT messages per direction between two agents
// until the user writes again (newRound), INBOX_LIMIT unread messages per agent, TEXT_LIMIT
// characters per message.
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { SendMessageArgs, WaitArgs } from "@neoworks-dev/neo-protocol/core-tools";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";

/** The session's own agent. */
export const MAIN = "main";
/** The app of a turn that starts because agents wrote to the session's agent. */
export const MESSAGES_APP = "agents";
export const PAIR_LIMIT = 20;
export const INBOX_LIMIT = 20;
export const TEXT_LIMIT = 20_000;
const DEFAULT_WAIT_SECONDS = 60;
const MAX_WAIT_SECONDS = 3600;
/** Ended subagents remembered per session, to say "has ended" instead of "no such agent". */
const REMEMBERED = 50;

/** A running subagent. */
export interface Member {
  id: string;
  /** Harness name: neo, claude, codex… */
  agent: string;
  /** The session it runs as, when it works in a directory of its own. */
  child?: string;
}

/** Who a caller is: `id` is `main` or a subagent of `room`'s session. */
export interface Address {
  room: string;
  id: string;
}

/** What the gateway calls for send_message, wait and the messages that ride on its results. */
export interface MessageTools {
  send: (session: string, guest: string | undefined, args: SendMessageArgs) => CallToolResult;
  wait: (
    session: string,
    guest: string | undefined,
    args: WaitArgs,
    signal?: AbortSignal,
  ) => Promise<CallToolResult>;
  /** Takes what waits for the caller (an ACP agent gets it with its next tool result). */
  take: (session: string, guest: string | undefined) => string[];
}

export interface MessengerParts {
  append: (session: string, event: SessionEvent) => void;
  /** A message came for the agent of this session: start its turn if it is idle. */
  wake: (session: string) => void;
}

interface Room {
  /** Running subagents by id. */
  members: Map<string, Member & { background: boolean }>;
  /** Unread messages by recipient (`main` or an id), already formatted. */
  inboxes: Map<string, string[]>;
  ended: Map<string, { agent: string; how: string }>;
  /** Messages sent since the user last wrote, by "from>to". */
  sent: Map<string, number>;
  /** Wake the waits of this session when something changes. */
  watchers: Set<() => void>;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/** The form an agent reads: `<message from="d_1a2b3c4d" agent="claude">…</message>`. */
function format(from: string, agent: string | undefined, body: string, reply: boolean): string {
  const attrs = [`from="${from}"`];
  if (agent !== undefined) attrs.push(`agent="${agent}"`);
  if (reply) attrs.push(`reply="true"`);
  return `<message ${attrs.join(" ")}>\n${body}\n</message>`;
}

export class Messenger implements MessageTools {
  private readonly rooms = new Map<string, Room>();
  /** The subagent that runs as a session of its own, by that session. */
  private readonly children = new Map<string, Address>();

  constructor(private readonly parts: MessengerParts) {}

  private room(id: string): Room {
    let room = this.rooms.get(id);
    if (!room) {
      room = {
        members: new Map(),
        inboxes: new Map(),
        ended: new Map(),
        sent: new Map(),
        watchers: new Set(),
      };
      this.rooms.set(id, room);
    }
    return room;
  }

  // ---- the subagents (delegate) ----

  /**
   * A subagent starts. One that wasn't started in the background blocks `main` until it ends,
   * so it can't write to it.
   */
  join(room: string, member: Member, background: boolean): void {
    this.room(room).members.set(member.id, { ...member, background });
    if (member.child !== undefined) {
      this.children.set(member.child, { room, id: member.id });
    }
  }

  /** The subagent is over; what was left in its inbox is dropped, later messages are refused. */
  leave(room: string, id: string, how: string): void {
    const r = this.rooms.get(room);
    const member = r?.members.get(id);
    if (!r || !member) return;
    r.members.delete(id);
    r.inboxes.delete(id);
    r.ended.set(id, { agent: member.agent, how });
    const [oldest] = r.ended.keys();
    if (r.ended.size > REMEMBERED && oldest !== undefined) r.ended.delete(oldest);
    if (member.child !== undefined) this.children.delete(member.child);
    this.notify(r);
  }

  /** Whether a subagent of this harness (not in a directory of its own, unless `child`) runs. */
  running(room: string, agent: string, child?: string): Member | undefined {
    for (const member of this.rooms.get(room)?.members.values() ?? []) {
      if (member.agent === agent && member.child === child) return member;
    }
    return undefined;
  }

  /** Takes what waits for a subagent. */
  takeOf(room: string, id: string): string[] {
    return this.rooms.get(room)?.inboxes.get(id)?.splice(0) ?? [];
  }

  hasPending(room: string, id: string): boolean {
    return (this.rooms.get(room)?.inboxes.get(id)?.length ?? 0) > 0;
  }

  /**
   * A background subagent's final reply, for `main`. Not counted against the limits. `wake`:
   * start a turn if `main` is idle (not for one the user stopped).
   */
  report(room: string, member: Member, body: string, wake: boolean): void {
    this.post(room, member.id, MAIN, body, { reply: true, wake, agent: member.agent });
  }

  /**
   * What happened beside `main` (`from`: background, a command it ran there ended), for it to
   * hear like a message; starts a turn if it is idle. Not counted against the limits.
   */
  tell(room: string, from: string, body: string): void {
    this.post(room, from, MAIN, body, { reply: false, wake: true });
  }

  /** The user wrote again: agents may message each other afresh. */
  newRound(room: string): void {
    this.rooms.get(room)?.sent.clear();
  }

  // ---- the callers (gateway) ----

  /**
   * Who a gateway call is from. `guest` (x-neo-guest) is a subagent of the harness it names,
   * working in the session; without it the caller is the session's own agent, or the subagent
   * that runs as that session. A guest that isn't a running subagent (the user's own `/claude
   * task` turn) is nobody.
   */
  identify(session: string, guest: string | undefined): Address | undefined {
    if (guest === undefined) return this.children.get(session) ?? { room: session, id: MAIN };
    const member = this.running(session, guest);
    return member && { room: session, id: member.id };
  }

  take(session: string, guest: string | undefined): string[] {
    const from = this.identify(session, guest);
    if (!from) return [];
    return this.takeOf(from.room, from.id);
  }

  send(session: string, guest: string | undefined, args: SendMessageArgs): CallToolResult {
    const from = this.identify(session, guest);
    if (!from) return text(NOT_A_PARTY, true);
    const room = this.room(from.room);
    const body = args.text.trim();
    if (body === "") return text("send_message: text is empty", true);
    if (body.length > TEXT_LIMIT) {
      return text(
        `The message is too long (${body.length} of ${TEXT_LIMIT} characters). Write the bulk to a file and send its path.`,
        true,
      );
    }
    const to = this.resolve(room, args.to);
    if ("error" in to) return text(to.error, true);
    if ("gone" in to) {
      const gone = room.ended.get(to.gone);
      const who = gone ? `${gone.agent} ${to.gone}` : to.gone;
      return text(`${who} has ended (${gone?.how ?? "over"}); it can't read messages.`, true);
    }
    if (to.id === from.id) return text("That's you: you can't message yourself.", true);
    const sender = room.members.get(from.id);
    if (to.id === MAIN && sender && !sender.background) {
      return text(
        "main is waiting for you to finish (it started you without background) and can't answer now. Put what it needs in your final reply.",
        true,
      );
    }
    const pair = `${from.id}>${to.id}`;
    const sent = room.sent.get(pair) ?? 0;
    const names = `${this.describe(room, from.id)} to ${this.describe(room, to.id)}`;
    if (sent >= PAIR_LIMIT) {
      return text(
        `Message limit reached: ${sent} messages from ${names} since the user last wrote. Stop messaging; finish your work, or ask the user.`,
        true,
      );
    }
    const unread = room.inboxes.get(to.id)?.length ?? 0;
    if (unread >= INBOX_LIMIT) {
      return text(
        `${this.describe(room, to.id)} has ${unread} unread messages. Wait for it to catch up.`,
        true,
      );
    }
    room.sent.set(pair, sent + 1);
    this.post(from.room, from.id, to.id, body, { reply: false, wake: true });
    return text(`Sent to ${this.describe(room, to.id)}.`);
  }

  async wait(
    session: string,
    guest: string | undefined,
    args: WaitArgs,
    signal?: AbortSignal,
  ): Promise<CallToolResult> {
    const me = this.identify(session, guest);
    if (!me) return text(NOT_A_PARTY, true);
    const room = this.room(me.room);
    const seconds = Math.min(
      Math.max(args.timeoutSeconds ?? DEFAULT_WAIT_SECONDS, 1),
      MAX_WAIT_SECONDS,
    );
    const watched: string[] = [];
    for (const name of args.for ?? []) {
      const found = this.resolve(room, name);
      if ("error" in found) return text(found.error, true);
      if ("gone" in found) watched.push(found.gone);
      else if (found.id !== MAIN) watched.push(found.id);
    }
    const deadline = Date.now() + seconds * 1000;
    for (;;) {
      const lines = this.takeOf(me.room, me.id);
      for (const id of watched) {
        if (room.members.has(id)) continue;
        const gone = room.ended.get(id);
        const how = gone ? `${gone.agent} ${id} has ended (${gone.how}).` : `${id} has ended.`;
        // Its final reply, if it was a background one, is among the messages.
        lines.push(how);
      }
      if (lines.length > 0) return text(lines.join("\n\n"));
      if (me.id === MAIN && room.members.size === 0) {
        return text("No subagent is running and nothing is waiting for you.");
      }
      const left = deadline - Date.now();
      if (left <= 0 || signal?.aborted === true) {
        return text(`Nothing new in ${seconds}s. ${this.roster(room)}`);
      }
      await this.changed(room, left, signal);
    }
  }

  // ---- inside ----

  /** Logs the message and queues it for the recipient. */
  private post(
    roomId: string,
    from: string,
    to: string,
    body: string,
    opts: { reply: boolean; wake: boolean; agent?: string },
  ): void {
    const room = this.room(roomId);
    const fromAgent = opts.agent ?? room.members.get(from)?.agent;
    const toMember = room.members.get(to);
    const event: Extract<SessionEvent, { type: "agent_message" }> = {
      type: "agent_message",
      from,
      to,
      text: body,
    };
    if (fromAgent !== undefined) event.fromAgent = fromAgent;
    if (toMember) event.toAgent = toMember.agent;
    if (opts.reply) event.reply = true;
    this.parts.append(roomId, event);
    const inbox = room.inboxes.get(to) ?? [];
    inbox.push(format(from, fromAgent, body, opts.reply));
    room.inboxes.set(to, inbox);
    this.notify(room);
    if (!opts.wake) return;
    // A subagent that runs as a session of its own is that session's agent.
    const session = to === MAIN ? roomId : toMember?.child;
    if (session !== undefined) this.parts.wake(session);
  }

  private notify(room: Room): void {
    for (const watcher of [...room.watchers]) watcher();
  }

  /** Resolves when something in the session changes, or after `ms`, or when `signal` aborts. */
  private changed(room: Room, ms: number, signal?: AbortSignal): Promise<void> {
    return new Promise((resolve) => {
      const done = (): void => {
        clearTimeout(timer);
        room.watchers.delete(done);
        signal?.removeEventListener("abort", done);
        resolve();
      };
      const timer = setTimeout(done, ms);
      room.watchers.add(done);
      signal?.addEventListener("abort", done, { once: true });
    });
  }

  /** `to` as an agent of the session: main, a subagent's id, or a harness name running once. */
  private resolve(room: Room, name: string): { id: string } | { gone: string } | { error: string } {
    const wanted = name.trim();
    if (wanted.toLowerCase() === MAIN) return { id: MAIN };
    if (room.members.has(wanted)) return { id: wanted };
    const named = [...room.members.values()].filter((m) => m.agent === wanted.toLowerCase());
    if (named.length === 1 && named[0]) return { id: named[0].id };
    if (named.length > 1) {
      const ids = named.map((m) => m.id).join(", ");
      return { error: `${named.length} ${wanted} subagents run (${ids}): name one by its id.` };
    }
    if (room.ended.has(wanted)) return { gone: wanted };
    return { error: `No running subagent "${wanted}". ${this.roster(room)}` };
  }

  /** "Running: d_1 (claude), d_2 (neo). main is the session's agent." */
  private roster(room: Room): string {
    const list = [...room.members.values()].map((m) => `${m.id} (${m.agent})`);
    const running = list.length > 0 ? `Running: ${list.join(", ")}.` : "No subagent is running.";
    return `${running} main is the session's own agent.`;
  }

  private describe(room: Room, id: string): string {
    if (id === MAIN) return MAIN;
    const member = room.members.get(id);
    return member ? `${id} (${member.agent})` : id;
  }
}

const NOT_A_PARTY =
  "send_message and wait are for the session's agent and the subagents it started with delegate.";

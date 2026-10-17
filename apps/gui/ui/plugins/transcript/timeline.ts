// What the session's events say about the chat's items that the items themselves do not: when
// each happened, which turn it belongs to and how that ended, and the tool calls' kind and input.
// Items carry none of it, so the transcript folds the events (timeline.svelte.ts) and looks the
// rest up by the item's `ref`. Plain functions, tested without a window.
import type {
  SessionEvent,
  SessionEventRecord,
  StopReason,
  ToolKind,
} from "@suite/protocol/types";

/** When an item first and last had something to say, in Unix milliseconds. */
export interface Stamp {
  first: number;
  last: number;
}

/** What the harness said about a tool call besides its title. */
export interface ToolFacts {
  kind?: ToolKind;
  tool?: string;
  rawInput?: unknown;
  /** The guest that made the call; its title carries the name as a prefix. */
  agent?: string;
}

/** One turn: from a user's message to the end of what the agent did for it. */
export interface TurnInfo {
  startedAt: number;
  endedAt?: number;
  stopReason?: StopReason;
  error?: string;
  /** The ref of the notice a turn that did not end normally leaves in the chat. */
  noticeRef?: string;
}

export interface Timeline {
  stamps: Map<string, Stamp>;
  facts: Map<string, ToolFacts>;
  /** By the ref of the turn's user message. */
  turns: Map<string, TurnInfo>;
  /** The turn that has not ended yet. */
  current?: string;
}

export function emptyTimeline(): Timeline {
  return { stamps: new Map(), facts: new Map(), turns: new Map() };
}

/** The ref the daemon gives the chat item an event draws or changes (daemon/src/view/session-view.ts). */
export function refOf(event: SessionEvent, seq: number): string | undefined {
  switch (event.type) {
    case "user_message":
      return `u${seq}`;
    case "agent_message_chunk":
      return `m-${event.messageId}`;
    case "agent_thought_chunk":
      return `t-${event.messageId}`;
    case "tool_call":
    case "tool_call_update":
      return `c-${event.toolCallId}`;
    case "plan":
      return event.agent === undefined ? "plan" : `plan-${event.agent}`;
    case "permission_request":
    case "permission_resolved":
      return `p-${event.id}`;
    case "choice":
    case "choice_made":
      return `q-${event.id}`;
    default:
      return undefined;
  }
}

/** The timeline after one more event. Changes `timeline` in place and returns it. */
export function applyRecord(timeline: Timeline, record: SessionEventRecord): Timeline {
  const { event, seq, ts } = record;
  const ref = refOf(event, seq);
  if (ref !== undefined) {
    const stamp = timeline.stamps.get(ref);
    if (stamp === undefined) timeline.stamps.set(ref, { first: ts, last: ts });
    else stamp.last = ts;
  }
  switch (event.type) {
    case "user_message":
      timeline.turns.set(`u${seq}`, { startedAt: ts });
      timeline.current = `u${seq}`;
      break;
    case "turn_ended": {
      const turn = timeline.current === undefined ? undefined : timeline.turns.get(timeline.current);
      if (turn === undefined) break;
      turn.endedAt = ts;
      turn.stopReason = event.stopReason;
      if (event.error !== undefined) turn.error = event.error;
      if (event.stopReason !== "end_turn") turn.noticeRef = `e${seq}`;
      timeline.current = undefined;
      break;
    }
    case "tool_call":
      timeline.facts.set(`c-${event.toolCallId}`, {
        kind: event.kind,
        tool: event.tool,
        rawInput: event.rawInput,
        agent: event.agent,
      });
      break;
    case "tool_call_update": {
      const facts = timeline.facts.get(`c-${event.toolCallId}`);
      if (facts === undefined) break;
      if (event.tool !== undefined) facts.tool = event.tool;
      if (event.rawInput !== undefined) facts.rawInput = event.rawInput;
      break;
    }
    default:
      break;
  }
  return timeline;
}

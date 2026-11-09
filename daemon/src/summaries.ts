// What a session overview shows besides state: a one-line preview of the latest message, the
// lines its edits added and removed, and what its agents used. Built from the log on first use, then kept current from
// appends. Also names untitled sessions after their first message, until a model writes a better
// title (titles.ts).
import type { SessionEvent, SessionUsage, Usage } from "@neoworks-dev/neo-protocol/types";
import { MESSAGES_APP } from "./messages.ts";
import type { SessionStore } from "./store.ts";
import { diffLines } from "./view/diff.ts";

const PREVIEW_CHARS = 120;
const TITLE_CHARS = 60;

export interface Summary {
  /** Only when it was just given (the session's row had none). */
  title?: string;
  preview?: string;
  added: number;
  removed: number;
  usage?: SessionUsage;
}

interface State {
  /** The latest agent message (any agent): id and text so far. */
  agent?: { id: string; text: string };
  /** The latest user message, when no agent message came after it. */
  user?: string;
  /** Per tool call and path, the last diff's counts (updates repeat earlier content). */
  diffs: Map<string, { added: number; removed: number }>;
  /** Per agent ("" for the session's own), its newest usage event; each carries running totals. */
  usage: Map<string, Usage>;
  /** The agent whose usage came last: its context is the one shown. */
  lastAgent?: string;
}

/** The first non-empty line, collapsed and cut. */
export function oneLine(text: string, max: number): string | undefined {
  const line = text
    .split("\n")
    .map((l) => l.replace(/\s+/g, " ").trim())
    .find((l) => l !== "");
  if (line === undefined) return undefined;
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

export class SessionSummaries {
  private readonly states = new Map<string, State>();

  constructor(private readonly store: SessionStore) {
    store.onAppend(({ session, event }) => {
      const state = this.states.get(session);
      if (state) apply(state, event);
      if (event.type === "user_message" && event.app !== MESSAGES_APP) {
        this.nameAfter(session, event.shown ?? event.text);
      }
    });
  }

  get(session: string): Summary {
    let state = this.states.get(session);
    let named: string | undefined;
    if (!state) {
      state = { diffs: new Map(), usage: new Map() };
      let first: string | undefined;
      for (const { event } of this.store.read(session)) {
        if (first === undefined && event.type === "user_message" && event.app !== MESSAGES_APP) {
          first = event.shown ?? event.text;
        }
        apply(state, event);
      }
      this.states.set(session, state);
      // Sessions from before titles were given get one now.
      if (first !== undefined) named = this.nameAfter(session, first);
    }
    const summary: Summary = { added: 0, removed: 0 };
    if (named !== undefined) summary.title = named;
    for (const counts of state.diffs.values()) {
      summary.added += counts.added;
      summary.removed += counts.removed;
    }
    const latest = state.agent?.text ?? state.user;
    const preview = latest === undefined ? undefined : oneLine(latest, PREVIEW_CHARS);
    if (preview !== undefined) summary.preview = preview;
    const usage = usageOf(state);
    if (usage !== undefined) summary.usage = usage;
    return summary;
  }

  /** Drops what is kept for a session that was deleted. */
  forget(session: string): void {
    this.states.delete(session);
  }

  /** An untitled session is named after its first message; the title given, if any. */
  private nameAfter(session: string, text: string): string | undefined {
    const row = this.store.getSession(session);
    if (!row || row.title !== undefined) return undefined;
    const title = oneLine(text, TITLE_CHARS);
    if (title !== undefined) this.store.setTitle(session, title, "first_line");
    return title;
  }
}

function apply(state: State, event: SessionEvent): void {
  switch (event.type) {
    case "user_message":
      // The turn that agents' messages start isn't something the user said.
      if (event.app === MESSAGES_APP) return;
      state.user = event.shown ?? event.text;
      state.agent = undefined;
      return;
    case "agent_message_chunk":
      if (state.agent?.id === event.messageId) state.agent.text += event.text;
      else state.agent = { id: event.messageId, text: event.text };
      return;
    case "tool_call_update":
      for (const content of event.content ?? []) {
        if (content.type !== "diff") continue;
        let added = 0;
        let removed = 0;
        for (const line of diffLines(content.oldText, content.newText)) {
          if (line.kind === "add") added++;
          else if (line.kind === "remove") removed++;
        }
        state.diffs.set(`${event.toolCallId}\0${content.path}`, { added, removed });
      }
      return;
    case "usage": {
      const agent = event.agent ?? "";
      // An agent reports what it knows; a number it leaves out keeps its last value.
      state.usage.set(agent, { ...state.usage.get(agent), ...event });
      state.lastAgent = agent;
      return;
    }
    default:
      return;
  }
}

function usageOf(state: State): SessionUsage | undefined {
  if (state.usage.size === 0) return undefined;
  const usage: SessionUsage = { cost: 0, inputTokens: 0, outputTokens: 0 };
  for (const agent of state.usage.values()) {
    usage.cost += agent.cost ?? 0;
    usage.inputTokens += agent.inputTokens ?? 0;
    usage.outputTokens += agent.outputTokens ?? 0;
  }
  const last = state.lastAgent === undefined ? undefined : state.usage.get(state.lastAgent);
  if (last?.contextUsed !== undefined) usage.contextUsed = last.contextUsed;
  if (last?.contextSize !== undefined) usage.contextSize = last.contextSize;
  return usage;
}

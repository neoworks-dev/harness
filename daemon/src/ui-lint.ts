// UI that runs long (docs/neoui.md): the library asks for a few words per title and label and a
// short line per subtitle. When a turn ends, the main agent's neoui blocks in it are measured, and
// what ran long is told to the agent with its next prompt (a user_note, not shown in the chat).
import type { SessionStore } from "./store.ts";

/** At most this many words, by what holds them; the library asks for fewer, this is past slack. */
const LIMITS = {
  /** A title, label or tag: a few words. */
  name: 8,
  /** A ListItem's subtitle, an OptionCard's description: one short line. */
  line: 12,
  /** A Callout or a Text: a sentence or two. */
  body: 45,
} as const;

/** Attributes and what they hold. */
const ATTRS: Record<string, keyof typeof LIMITS> = {
  title: "name",
  label: "name",
  subtitle: "line",
  description: "line",
};

/** Components whose text between the tags is a body. */
const BODIES = new Set(["Callout", "Text"]);

/** How many findings one note names. */
const MAX_FINDINGS = 3;

export interface Finding {
  component: string;
  /** The attribute, or "text" for what is between the tags. */
  part: string;
  words: number;
  limit: number;
  text: string;
}

const FENCE = /```neoui[^\n]*\n([\s\S]*?)(?:```|$)/g;
// A start tag: its name, then attributes, whose values may hold `>` inside quotes or braces.
const TAG = /<([A-Za-z][\w.]*)((?:[^>"'{]|"[^"]*"|'[^']*'|\{[^}]*\})*)>/g;
const ATTR = /\b([a-z]+)="([^"]*)"/g;

function words(text: string): number {
  return text.split(/\s+/).filter((w) => w !== "").length;
}

/** What runs long in the neoui blocks of `text`, longest past its limit first. */
export function lintUi(text: string): Finding[] {
  const found: Finding[] = [];
  for (const [, block = ""] of text.matchAll(FENCE)) {
    for (const tag of block.matchAll(TAG)) {
      const [whole, component = "", attrs = ""] = tag;
      for (const [, name = "", value = ""] of attrs.matchAll(ATTR)) {
        const kind = ATTRS[name];
        if (kind === undefined) continue;
        const n = words(value);
        if (n > LIMITS[kind]) {
          found.push({ component, part: name, words: n, limit: LIMITS[kind], text: value });
        }
      }
      if (!BODIES.has(component) || whole.endsWith("/>")) continue;
      const start = (tag.index ?? 0) + whole.length;
      const end = block.indexOf(`</${component}>`, start);
      if (end < 0) continue;
      const body = block.slice(start, end).replace(/<[^>]*>/g, " ");
      const n = words(body);
      if (n > LIMITS.body) {
        found.push({ component, part: "text", words: n, limit: LIMITS.body, text: body.trim() });
      }
    }
  }
  return found.sort((a, b) => b.words / b.limit - a.words / a.limit);
}

/** The note for `found`: what ran long and how long it may be. */
export function lintNote(found: Finding[]): string {
  const lines = found.slice(0, MAX_FINDINGS).map((f) => {
    const start = f.text.split(/\s+/).slice(0, 6).join(" ");
    const what = f.part === "text" ? `${f.component}'s text` : `a ${f.component}'s ${f.part}`;
    return `- ${what} ("${start}…") is ${f.words} words; keep it under ${f.limit}.`;
  });
  if (found.length > MAX_FINDINGS) lines.push(`- and ${found.length - MAX_FINDINGS} more.`);
  return (
    "Your last reply's UI ran long. UI is read at a glance; the detail goes in the prose around " +
    `it.\n${lines.join("\n")}`
  );
}

/** Measures each turn's UI as it ends and notes what ran long. */
export class UiLint {
  /** The main agent's text of the turn under way and its last message, per session. */
  private readonly turns = new Map<string, { text: string; message: string }>();

  constructor(private readonly store: SessionStore) {
    store.onAppend(({ session, event }) => {
      if (event.type === "user_message") {
        this.turns.delete(session);
      } else if (event.type === "agent_message_chunk" && event.agent === undefined) {
        const turn = this.turns.get(session) ?? { text: "", message: event.messageId };
        // Each message on lines of its own.
        if (turn.message !== event.messageId) turn.text += "\n";
        turn.text += event.text;
        turn.message = event.messageId;
        this.turns.set(session, turn);
      } else if (event.type === "turn_ended") {
        const text = this.turns.get(session)?.text ?? "";
        this.turns.delete(session);
        const found = lintUi(text);
        if (found.length === 0) return;
        // Not from inside the store's own append.
        queueMicrotask(() => {
          if (this.store.getSession(session) === undefined) return;
          this.store.append(session, { type: "user_note", note: lintNote(found), about: "neoui" });
        });
      }
    });
  }
}

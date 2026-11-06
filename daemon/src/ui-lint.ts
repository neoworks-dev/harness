// UI that runs long or is written wrong (docs/neoui.md). The library asks for a few words per title
// and label and a short line per subtitle; an agent that knows the components by name only (it
// didn't look them up with ui_library) may guess an attribute. When a turn ends, the main agent's neoui blocks in it are
// measured and evaluated, and what ran long and what was wrong are told to the agent with its next
// prompt (a user_note, not shown in the chat), a mistake with what its component takes.
import {
  type AppComponents,
  COMPONENTS,
  appLibrary,
  asComponent,
  check,
  signature,
} from "@neoworks/neoui";
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

/**
 * Components whose titles and names are quoted, not written: an issue's, a commit's subject, a
 * test's name. They are as long as they are; the components cut them to fit.
 */
const QUOTED = new Set([
  "Issue",
  "PullRequest",
  "Commit",
  "Test",
  "Check",
  "File",
  "Frame",
  "CodeRef",
  "TimelineItem",
]);

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
      if (QUOTED.has(component)) continue;
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

/** A mistake neoui found in a block: its message, and the component it is about. */
export interface Mistake {
  message: string;
  component?: string;
}

/** What neoui finds wrong in the neoui blocks of `text` (the first few). */
export function mistakesIn(text: string, apps: AppComponents = {}): Mistake[] {
  const library = appLibrary(apps);
  const out: Mistake[] = [];
  for (const [, block = ""] of text.matchAll(FENCE)) {
    if (block.trim() === "") continue;
    for (const error of check(block, { apps: library }).doc.errors ?? []) {
      const mistake: Mistake = { message: error.message };
      const named = componentOf(error.message);
      if (named !== undefined) mistake.component = named;
      out.push(mistake);
      if (out.length >= MAX_FINDINGS) return out;
    }
  }
  return out;
}

/** The note for `wrong`: each mistake with what its component takes, or the nearest name. */
export function mistakesNote(wrong: Mistake[], apps: AppComponents = {}): string {
  const known = new Map(COMPONENTS.map((c) => [c.name, c]));
  for (const reg of appLibrary(apps).components.values()) known.set(reg.call, asComponent(reg));
  const lines = wrong.map((m) => {
    const c = m.component === undefined ? undefined : known.get(m.component);
    if (c !== undefined) return `- ${m.message}. It takes: ${signature(c)}`;
    const near = m.component === undefined ? undefined : nearest(m.component, [...known.keys()]);
    return near === undefined ? `- ${m.message}.` : `- ${m.message}; did you mean ${near}?`;
  });
  return `Your last reply's UI had mistakes; the app drew what it could. For next time:\n${lines.join("\n")}`;
}

/** The component an error of neoui's is about: `Unknown component X`, `X.attr is …`, `X cannot be inside`. */
function componentOf(message: string): string | undefined {
  for (const pattern of [
    /^Unknown component ([\w.]+)/,
    /^([\w.]+)\.[\w-]+ (?:is|are) /,
    /^([\w.]+) cannot be inside/,
  ]) {
    const named = pattern.exec(message)?.[1];
    if (named !== undefined) return named;
  }
  return undefined;
}

/** The name closest to `name` (by edits), when it is close enough to be a typo of it. */
function nearest(name: string, names: string[]): string | undefined {
  let best: string | undefined;
  let bestCost = Math.max(2, Math.floor(name.length / 3)) + 1;
  for (const candidate of names) {
    const cost = edits(name.toLowerCase(), candidate.toLowerCase());
    if (cost < bestCost) {
      best = candidate;
      bestCost = cost;
    }
  }
  return best;
}

function edits(a: string, b: string): number {
  let row = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const next = [i];
    for (let j = 1; j <= b.length; j++) {
      const swap = (row[j - 1] ?? 0) + (a[i - 1] === b[j - 1] ? 0 : 1);
      next.push(Math.min((row[j] ?? 0) + 1, (next[j - 1] ?? 0) + 1, swap));
    }
    row = next;
  }
  return row[b.length] ?? 0;
}

/** Measures each turn's UI as it ends and notes what ran long and what was wrong. */
export class UiLint {
  /** The main agent's text of the turn under way and its last message, per session. */
  private readonly turns = new Map<string, { text: string; message: string }>();

  constructor(
    private readonly store: SessionStore,
    /** The components the open apps add: a block may use them. */
    private readonly apps: () => AppComponents = () => ({}),
  ) {
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
        const notes: string[] = [];
        const found = lintUi(text);
        if (found.length > 0) notes.push(lintNote(found));
        const wrong = mistakesIn(text, this.apps());
        if (wrong.length > 0) notes.push(mistakesNote(wrong, this.apps()));
        if (notes.length === 0) return;
        // Not from inside the store's own append.
        queueMicrotask(() => {
          if (this.store.getSession(session) === undefined) return;
          for (const note of notes)
            this.store.append(session, { type: "user_note", note, about: "neoui" });
        });
      }
    });
  }
}

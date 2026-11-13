// Session titles written by a model. After a session's first message one cheap model call writes
// a short title for it, which replaces the start of the message that names the session until
// then; every REFRESH_EVERY messages and on `/title` it is written again from the conversation
// so far, as the conversation moves on. A title the user gave the session
// (session.create's) is never replaced by itself, only by `/title`.
//
// The model is the user's choice (`/title-model <harness> [model]`, kept for every session). Until
// they choose it is neo's: DEFAULT_TITLE_MODEL when that harness exists here, else the
// daemon's own agent on the model it is configured with. When a model can't be reached the session
// keeps the start of its first message.
import type { GenerateTextParams } from "@neoworks-dev/neo/agent";
import { BadRequestError } from "./focus.ts";
import type { Logger } from "./log.ts";
import type { Settings, TitleModel } from "./settings.ts";
import type { SessionStore } from "./store.ts";

/** What neo asks for titles until the user chooses: Claude Code's Haiku. */
export const DEFAULT_TITLE_MODEL: TitleModel = { harness: "claude", model: "haiku" };

/** A title a model wrote is looked at again after this many more user messages. */
export const REFRESH_EVERY = 10;

/** How many times a title is asked for before the session keeps the one it has. */
export const TITLE_ATTEMPTS = 3;

/** A title is cut to this many characters (the model is asked for fewer than 40). */
const TITLE_CHARS = 60;

/** How long one model call may take. */
const TITLE_TIMEOUT_MS = 90_000;

/** How much of the first message the model reads: its start says what it is about. */
const MESSAGE_CHARS = 1000;

/** How much of the user's messages a new title is written from, and of each one. */
const CONVERSATION_CHARS = 1500;
const TURN_CHARS = 300;

const SYSTEM = `You write titles for conversations between a user and a coding agent, so the user can recognise a conversation weeks later.
Reply with a JSON object and nothing else: {"title": "..."}.

Before answering, silently reduce the request to:
- Subject: what system, feature or problem it is really about.
- Outcome: what the user ultimately wants to understand or change.
- Incidental instructions: what only describes how the agent should do the work.
Title the subject and outcome. Leave the incidental instructions out.

Rules:
- 3-8 words, fewer than 40 characters.
- A compact noun phrase or a clear action phrase, in the language the user writes in.
- When the request lists several symptoms or steps, name the umbrella goal.
- Do not copy and truncate the message.
- Do not claim the work is done.
- No quotes, labels, filler or trailing punctuation.
- Models, tools, output formats and how to work only belong in the title when they are the topic.
- Do not use tools or answer the request; only write the title.`;

const REGENERATE = `
This is a new title for a conversation that has one already, because it has moved on or the old one did not fit.
You get the user's messages: the subject stays the subject until the user clearly changes what the conversation is about, and a conversation that moves from research to planning to implementation to review has usually not changed subjects.
Keep the previous title when it is still accurate. Otherwise return a clearly better one, not a paraphrase.`;

/** Titles a model returns when it has nothing to say. */
const GENERIC = /^(new|untitled)(\s+(thread|session|chat|conversation))?$/i;

/** The title in a model's reply: `{"title": "…"}` (as asked), else a lone short line; cleaned. */
export function parseTitle(reply: string): string | undefined {
  let title: string | undefined;
  const object = /\{[\s\S]*\}/.exec(reply)?.[0];
  if (object !== undefined) {
    try {
      const parsed: unknown = JSON.parse(object);
      if (typeof parsed === "object" && parsed !== null && "title" in parsed) {
        if (typeof parsed.title === "string") title = parsed.title;
      }
    } catch {
      // Not JSON after all.
    }
  } else if (!reply.trim().includes("\n")) {
    title = reply;
  }
  return title === undefined ? undefined : cleanTitle(title);
}

/** One line without quotes or closing punctuation, cut at a word; undefined when it says nothing. */
export function cleanTitle(text: string): string | undefined {
  const title = text.replace(/\s+/g, " ").replace(/^[\s"'`“”‘’]+|[\s"'`“”‘’.:;,!]+$/g, "");
  if (title === "" || GENERIC.test(title)) return undefined;
  if (title.length <= TITLE_CHARS) return title;
  const cut = title.slice(0, TITLE_CHARS - 1);
  const space = cut.lastIndexOf(" ");
  return `${(space > TITLE_CHARS / 2 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

function clip(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max)}…`;
}

export interface TitleParts {
  store: SessionStore;
  settings: Settings;
  log: Logger;
  /** The daemon's own agent's harness name. */
  main: string;
  /** Whether the harness can write text: the daemon's agent if it can, or a guest. */
  can: (harness: string) => boolean;
  /** One model call on a harness. */
  generate: (harness: string, params: GenerateTextParams) => Promise<string>;
  /** A session's title changed: tell the clients. */
  changed: (session: string) => void;
  /** Whether a first message asks for a title by itself; `/title` works either way. */
  auto: boolean;
}

export class Titles {
  private readonly running = new Map<string, Promise<void>>();

  constructor(private readonly parts: TitleParts) {
    parts.store.onAppend(({ session, event }) => {
      if (event.type === "user_message" && parts.auto) this.afterMessage(session);
    });
  }

  /** The model that writes titles now, and whether the user chose it. */
  current(): { model: TitleModel; chosen: boolean } {
    const chosen = this.parts.settings.titleModel();
    if (chosen !== undefined) return { model: chosen, chosen: true };
    const suggested = this.parts.can(DEFAULT_TITLE_MODEL.harness)
      ? DEFAULT_TITLE_MODEL
      : { harness: this.parts.main };
    return { model: suggested, chosen: false };
  }

  /**
   * Writes the session's title again from the conversation so far, replacing the one it has
   * (also one the user gave). Resolves once it is set; rejects with why it could not be.
   */
  async regenerate(session: string): Promise<void> {
    if (!this.parts.store.getSession(session)) throw new BadRequestError(`no session ${session}`);
    if (this.parts.store.lastSeqOfType(session, "user_message") === 0) {
      throw new BadRequestError("there is nothing to title yet");
    }
    await this.write(session, "again");
  }

  /** The first message asks for a title; every REFRESH_EVERY-th asks for a model's again. */
  private afterMessage(session: string): void {
    const { store, log } = this.parts;
    const row = store.getSession(session);
    if (!row) return;
    // The user's own title stays.
    if (row.titleSource === undefined && row.title !== undefined) return;
    const count = store.readTypes(session, ["user_message"]).length;
    let kind: "first" | "again";
    if (row.titleSource !== "generated" && count === 1) kind = "first";
    else if (row.titleSource === "generated" && count % REFRESH_EVERY === 0) kind = "again";
    else return;
    if (!this.parts.can(this.current().model.harness)) return;
    this.write(session, kind).catch((err: unknown) => {
      log.warn(`title for ${session}: ${err instanceof Error ? err.message : String(err)}`);
    });
  }

  private write(session: string, kind: "first" | "again"): Promise<void> {
    const running = this.running.get(session);
    if (running) return running;
    const work = this.ask(session, kind).finally(() => this.running.delete(session));
    this.running.set(session, work);
    return work;
  }

  private async ask(session: string, kind: "first" | "again"): Promise<void> {
    const { store, generate, can } = this.parts;
    const { model } = this.current();
    if (!can(model.harness)) {
      throw new BadRequestError(
        `${model.harness} can't write titles; choose another: /title-model`,
      );
    }
    const params = this.prompt(session, kind);
    const reasons: string[] = [];
    for (let attempt = 1; attempt <= TITLE_ATTEMPTS; attempt++) {
      try {
        const request: GenerateTextParams = {
          ...params,
          signal: AbortSignal.timeout(TITLE_TIMEOUT_MS),
        };
        if (model.model !== undefined) request.model = model.model;
        const text = await generate(model.harness, request);
        const title = parseTitle(text);
        if (title === undefined) {
          reasons.push(`no title in the reply: ${clip(text.trim(), 80)}`);
          continue;
        }
        // The user may have named the session while the model worked.
        const row = store.getSession(session);
        if (kind === "first" && row?.titleSource === undefined && row?.title !== undefined) return;
        store.setTitle(session, title, "generated");
        this.parts.changed(session);
        return;
      } catch (err) {
        reasons.push(err instanceof Error ? err.message : String(err));
      }
    }
    const used = model.model === undefined ? model.harness : `${model.harness} ${model.model}`;
    throw new BadRequestError(`${used} did not write a title: ${reasons.at(-1) ?? "no reply"}`);
  }

  /** What to ask: the first message, or the conversation so far and the title it has. */
  private prompt(session: string, kind: "first" | "again"): { system: string; prompt: string } {
    const { store } = this.parts;
    const messages: string[] = [];
    for (const { event } of store.readTypes(session, ["user_message"])) {
      if (event.type === "user_message") messages.push(event.text);
    }
    if (kind === "first") {
      const text = messages[0] ?? "";
      return { system: SYSTEM, prompt: `The first message:\n\n${clip(text, MESSAGE_CHARS)}` };
    }
    const previous = store.getSession(session)?.title;
    let system = SYSTEM + REGENERATE;
    if (previous !== undefined) system += `\nThe previous title was ${JSON.stringify(previous)}.`;
    return { system, prompt: `The user's messages:\n\n${fit(messages).join("\n\n")}` };
  }
}

/** The messages, in order: the first, which says what it was about, and the latest that fit. */
function fit(messages: readonly string[]): string[] {
  const [first, ...rest] = messages.map((text) => clip(text, TURN_CHARS));
  if (first === undefined) return [];
  const latest: string[] = [];
  let size = first.length;
  for (const message of rest.reverse()) {
    if (size + message.length > CONVERSATION_CHARS) break;
    size += message.length;
    latest.unshift(message);
  }
  return [first, ...latest];
}

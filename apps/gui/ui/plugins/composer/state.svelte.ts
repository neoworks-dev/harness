// What the input side holds: the draft, the `/` menu, the history and the question panel (a
// permission or choice card answered in place of the input). The components draw it and the
// plugin's keys call it, so they agree on what is open.
import type { Context } from "@neoworks/extension-system";
import { clipboard, window } from "@neoworks/svelte-gpui";
import type { ViewItem, WaitingInput } from "@neoworks-dev/neo-protocol/types";
import { onAttachments, pasteAttachment, pickAttachments } from "../../kernel/attachments.ts";
import type { CommandsService } from "../../kernel/services/commands.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import {
  completion,
  isSlashWord,
  lookup,
  matching,
  moveSelection,
  type Command,
} from "../../lib/commands.ts";
import { History, walksHistory } from "./history.ts";
import {
  activePrompt,
  argsWithNote,
  copyText,
  focusAfterFilter,
  isLongChoice,
  matchingButtons,
  moveFocus,
  promptEngaged,
  startButton,
  takesNote,
  TYPING_PAUSE_MS,
  type Prompt,
} from "./prompt.ts";
import { appendHistory, loadHistory } from "./history-store.ts";
import { queueView, redraftImages, redraftText } from "./queue.ts";

/** The services the composer reads and calls, from the plugin that started it. */
interface Services {
  daemon: DaemonService;
  commands: CommandsService;
  /** Comments on code, from the viewer, to go with the next message. */
  comments: Context["comments"];
}

const NO_ITEMS: readonly ViewItem[] = [];
const NO_COMMANDS: readonly Command[] = [];
const NO_WAITING: readonly WaitingInput[] = [];
const NO_COMMENTS: Context["comments"]["list"] = [];

/** How long "copied" stays in the status line. */
const COPIED_NOTICE_MS = 1500;

/** How long a second Ctrl-C quits. */
const QUIT_WINDOW_MS = 2000;

class Composer {
  private services = $state.raw<Services>();
  /** The draft in the input. Kept while a question has the input's place. */
  text = $state("");
  /** Files picked with the paperclip, sent with the next message. */
  attachments = $state.raw<readonly string[]>([]);
  history = $state.raw(new History());
  /** The `/` menu's highlighted row. */
  selected = $state(0);
  /** The text the menu was closed (Esc) for; typing more opens it again. */
  menuClosedFor = $state<string>();
  helpOpen = $state(false);
  /** A short message for the status line, such as "press ctrl-c again to quit". */
  notice = $state<string>();

  /** Questions put aside with Esc; they stay in the chat. */
  dismissed = $state.raw<ReadonlySet<string>>(new Set());
  /** The question that has the input's place now. */
  engaged = $state(false);
  /** When the user last typed, for the pause before a question takes over. */
  typedAt = $state(0);
  /** The focused button of the question. */
  button = $state(0);
  /** What was typed to find an option of a long choice. */
  filter = $state("");
  /** The note being typed for the focused button's answer; undefined while there is none. */
  note = $state<string>();

  /** The oldest open question that was not put aside. */
  prompt = $derived(activePrompt(this.items, this.dismissed));
  /** The question is a choice long enough to be filtered by typing. */
  filtering = $derived(this.prompt !== undefined && isLongChoice(this.prompt.buttons));
  /** The positions of the options that match the filter (every option when it isn't filtered). */
  shown = $derived(this.shownButtons());
  /** The question is open but the user is typing, so the input stays (with a hint above). */
  waiting = $derived(this.prompt !== undefined && !this.engaged);
  menuVisible = $derived(
    !this.engaged &&
      !this.helpOpen &&
      isSlashWord(this.text) &&
      this.menuClosedFor !== this.text &&
      matching(this.text, this.menuEntries).length > 0,
  );
  /** The commands the menu lists, if it is open. */
  commands = $derived(this.listCommands());

  private quitArmedUntil = 0;
  private noticeTimer?: ReturnType<typeof setTimeout>;

  /** Serves the services until the returned call stops it. */
  start(services: Services): () => void {
    this.services = services;
    // Files dropped on the window, or an image pasted into it (a browser tab gets them as events).
    const stopFiles = onAttachments(
      (paths) => this.addAttachments(paths),
      (error) => this.attachFailed(error),
    );
    return () => {
      stopFiles();
      this.services = undefined;
    };
  }

  /** The comments on code waiting for the next message. */
  get comments(): Context["comments"]["list"] {
    return this.services?.comments.list ?? NO_COMMENTS;
  }

  private get items(): readonly ViewItem[] {
    return this.services?.daemon.items ?? NO_ITEMS;
  }

  /** Every command the menu can list. */
  private get menuEntries(): readonly Command[] {
    return this.services?.commands.list ?? NO_COMMANDS;
  }

  private listCommands(): Command[] {
    if (!this.menuVisible) return [];
    return matching(this.text, this.menuEntries);
  }

  // ---- typing ----

  /** The text changed by typing (not by the history or a completion). */
  typed(): void {
    this.typedAt = Date.now();
    this.history.reset();
    this.selected = 0;
  }

  // ---- sending ----

  /**
   * Enter: the highlighted command when the menu is open, else the draft. Alt-Enter or Ctrl-Enter
   * (`steer`) sends it into the running turn at once instead of after it, as the terminal does.
   */
  submit(steer = false): void {
    if (this.menuVisible) {
      const command = this.commands[Math.min(this.selected, this.commands.length - 1)];
      if (command) this.choose(command);
      return;
    }
    const text = this.text.trim();
    if (text === "" && this.attachments.length === 0 && this.comments.length === 0) return;
    this.text = "";
    if (this.attachments.length > 0) {
      this.sendWithAttachments(text, steer);
      return;
    }
    const command = text.startsWith("/") ? lookup(text, this.menuEntries) : undefined;
    if (command?.run) command.run();
    else this.send(text, steer);
  }

  /** Takes a comment back: the chip's button. */
  uncomment(id: string): void {
    this.services?.comments.remove(id);
  }

  /** `text` with the comments on code after it. A `/command` for the daemon leaves them for later. */
  private withComments(text: string): string {
    if (text.startsWith("/")) return text;
    return this.services?.comments.take(text) ?? text;
  }

  /** A menu pick (Enter or a press): runs the command, or leaves room for its arguments. */
  choose(command: Command): void {
    this.selected = 0;
    if (command.needsArgs) {
      this.text = completion(command);
      return;
    }
    this.text = "";
    if (command.run) command.run();
    else this.send(`/${command.name}`, false);
  }

  /** Tab: puts the highlighted command into the input with a space for arguments. */
  complete(): void {
    const command = this.commands[Math.min(this.selected, this.commands.length - 1)];
    if (!command) return;
    this.text = completion(command);
    this.selected = 0;
  }

  moveMenu(delta: number): void {
    this.selected = moveSelection(this.selected, delta, this.commands.length);
  }

  /** Picks files to send with the next message. */
  async attach(): Promise<void> {
    try {
      const picked = await pickAttachments();
      if (picked) this.addAttachments(picked);
    } catch (error) {
      this.attachFailed(error);
    }
  }

  private addAttachments(paths: readonly string[]): void {
    this.attachments = [...new Set([...this.attachments, ...paths])];
  }

  private attachFailed(error: unknown): void {
    if (this.services)
      this.services.daemon.error = error instanceof Error ? error.message : String(error);
  }

  /**
   * Ctrl-V: attaches the clipboard's image, if it holds one. Text still pastes into the input as
   * usual; this only adds the image.
   */
  async pasteImage(): Promise<void> {
    const path = await pasteAttachment();
    if (path) this.attachments = [...this.attachments, path];
  }

  detach(path: string): void {
    this.attachments = this.attachments.filter((attached) => attached !== path);
  }

  /** Images go along as images; other files are named in the text for the agent to read. */
  private sendWithAttachments(text: string, steer: boolean): void {
    const { images, others } = splitAttachments(this.attachments);
    this.attachments = [];
    const named = others.map((path) => `@${path}`).join("\n");
    const full = [text, named].filter((part) => part !== "").join("\n\n");
    if (text !== "") {
      const session = this.services?.daemon.sessionId;
      if (this.history.push(text) && session !== undefined) void appendHistory(session, text);
    }
    void this.services?.daemon.send(this.withComments(full), images, steer);
  }

  private send(text: string, steer: boolean): void {
    // Sending ends the typing: a question it brings (/model's choice) takes over at once.
    this.typedAt = 0;
    const session = this.services?.daemon.sessionId;
    if (this.history.push(text) && session !== undefined) void appendHistory(session, text);
    void this.services?.daemon.send(this.withComments(text), undefined, steer);
  }

  // ---- the queue ----

  /** The rows for the messages waiting behind the running turn, oldest first. */
  queue = $derived(queueView(this.services?.daemon.session?.waiting ?? NO_WAITING));
  private unqueuing = false;

  /**
   * Takes the user's queued messages back into the draft (Up in an empty input, or a press on
   * the queue): their text one per paragraph before what was typed meanwhile, their images
   * attached. Returns whether there was anything to take back.
   */
  async unqueue(): Promise<boolean> {
    const daemon = this.services?.daemon;
    if (!daemon || !this.queue.editable || this.unqueuing) return false;
    this.unqueuing = true;
    try {
      const { session, inputs } = await daemon.unqueue();
      if (inputs.length === 0 || daemon.sessionId !== session) return false;
      this.text = redraftText(inputs, this.text);
      this.addAttachments(redraftImages(inputs));
      this.history.reset();
      return true;
    } finally {
      this.unqueuing = false;
    }
  }

  /** Up in an empty input, off the menu and the history: takes the queue back. */
  get canUnqueue(): boolean {
    const empty = this.text.trim() === "";
    return empty && !this.menuVisible && !this.history.browsing && this.queue.editable;
  }

  // ---- history ----

  /** Up or Down in the input. Returns whether it was used for the history. */
  walkHistory(direction: "older" | "newer", columns: number, forced: boolean): boolean {
    if (!walksHistory(this.text, columns, forced)) return false;
    const text = direction === "older" ? this.history.older(this.text) : this.history.newer();
    if (text === undefined) return false;
    this.text = text;
    return true;
  }

  /** Loads the saved history when another session is shown. */
  async switchSession(session: string | undefined): Promise<void> {
    this.text = "";
    this.dismissed = new Set();
    this.note = undefined;
    this.history = new History();
    if (session === undefined) return;
    const loaded = await loadHistory(session);
    if (this.services?.daemon.sessionId === session) this.history = loaded;
  }

  // ---- the question panel ----

  /**
   * Settles `engaged` once the draft is gone and the pause in typing is over; returns how long to
   * wait, if more. A draft waits for the user, not for a timer.
   */
  syncEngaged(): number | undefined {
    const drafting = this.text.trim() !== "";
    const engaged = promptEngaged({
      hasPrompt: this.prompt !== undefined,
      engaged: this.engaged,
      drafting,
      now: Date.now(),
      typedAt: this.typedAt,
    });
    if (engaged !== this.engaged) this.engaged = engaged;
    if (this.prompt === undefined || engaged || drafting) return undefined;
    return Math.max(0, TYPING_PAUSE_MS - (Date.now() - this.typedAt));
  }

  /** The user asked for the waiting question (pressed its notice): it takes the input's place now. */
  answerWaiting(): void {
    if (this.prompt !== undefined) this.engaged = true;
  }

  /** A new question starts on its current (else its first) button. */
  startPrompt(prompt: Prompt | undefined): void {
    this.button = prompt ? startButton(prompt.buttons) : 0;
    this.note = undefined;
    this.filter = "";
  }

  private shownButtons(): number[] {
    const prompt = this.prompt;
    if (!prompt) return [];
    if (!this.filtering) return prompt.buttons.map((_, index) => index);
    return matchingButtons(prompt.buttons, this.filter);
  }

  /** The filter changed (typed or cleared): the focus goes to the first match. */
  setFilter(filter: string): void {
    this.filter = filter;
    const at = this.prompt && focusAfterFilter(this.prompt.buttons, filter);
    if (at !== undefined) this.button = at;
  }

  moveButton(delta: number): void {
    if (!this.prompt) return;
    if (this.filtering) this.button = moveFocus(this.shown, this.button, delta);
    else this.button = moveSelection(this.button, delta, this.prompt.buttons.length);
  }

  /** Enter: answers with the focused option, unless the filter leaves none. */
  pressFocused(): void {
    if (this.filtering && !this.shown.includes(this.button)) return;
    this.press(this.button);
  }

  /** Tab on a button that takes a note opens the note field; on others it is "next button". */
  tab(): void {
    if (this.filtering && !this.shown.includes(this.button)) return;
    if (takesNote(this.prompt?.buttons[this.button])) this.note = "";
    else this.moveButton(1);
  }

  /** Answers with button `index` (and the note typed for it, if any). */
  press(index: number): void {
    const prompt = this.prompt;
    const button = prompt?.buttons[index];
    if (!prompt || !button) return;
    // A copy button is the client's to handle: nothing goes to the daemon and the question stays.
    const copied = copyText(button);
    if (copied !== undefined) {
      void clipboard.write(copied);
      this.say("copied", COPIED_NOTICE_MS);
      return;
    }
    const args = this.note === undefined ? button.args : argsWithNote(button, this.note);
    this.note = undefined;
    this.dismissed = new Set([...this.dismissed, prompt.ref]);
    void this.services?.daemon.action(prompt.ref, button.action, args);
  }

  /** Brings back the questions put aside; false when that brought none back. */
  reopenPrompts(): boolean {
    const before = this.prompt?.ref;
    this.dismissed = new Set();
    const after = this.prompt?.ref;
    return after !== undefined && after !== before;
  }

  // ---- Esc and Ctrl-C ----

  /** Esc closes what is open, innermost first. Returns false when nothing was. */
  escape(): boolean {
    if (this.helpOpen) this.helpOpen = false;
    else if (this.menuVisible) this.menuClosedFor = this.text;
    else if (this.note !== undefined) this.note = undefined;
    else if (this.engaged && this.filtering && this.filter !== "") this.setFilter("");
    else if (this.prompt) this.dismissed = new Set([...this.dismissed, this.prompt.ref]);
    else return false;
    return true;
  }

  /** Ctrl-C: the first press asks for a second, which quits. */
  ctrlC(): void {
    const now = Date.now();
    if (now < this.quitArmedUntil) {
      window.close();
      return;
    }
    this.quitArmedUntil = now + QUIT_WINDOW_MS;
    this.say("press ctrl-c again to quit", QUIT_WINDOW_MS);
  }

  /** Any other key disarms the quit. */
  otherKey(): void {
    if (this.quitArmedUntil === 0) return;
    this.quitArmedUntil = 0;
    if (this.notice?.startsWith("press ctrl-c")) this.notice = undefined;
  }

  private say(text: string, ms: number): void {
    clearTimeout(this.noticeTimer);
    this.notice = text;
    this.noticeTimer = setTimeout(() => (this.notice = undefined), ms);
  }
}

export const composer = new Composer();

const IMAGE = /\.(png|jpe?g|gif|webp)$/i;

/** Whether the agent gets an attached file as an image (PNG, JPEG, GIF, WebP). */
export function isImage(path: string): boolean {
  return IMAGE.test(path);
}

/** The attached files the agent gets as images, and the rest. */
export function splitAttachments(paths: readonly string[]): { images: string[]; others: string[] } {
  return {
    images: paths.filter(isImage),
    others: paths.filter((path) => !isImage(path)),
  };
}

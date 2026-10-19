// What the input side holds: the draft, the `/` menu, the history and the question panel (a
// permission or choice card answered in place of the input). The components draw it and the
// plugin's keys call it, so they agree on what is open.
import { clipboard, window } from "@neoworks/svelte-gpui";
import type { ViewItem } from "@suite/protocol/types";
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

/** The services the composer reads and calls, from the plugin that started it. */
interface Services {
  daemon: DaemonService;
  commands: CommandsService;
}

const NO_ITEMS: readonly ViewItem[] = [];
const NO_COMMANDS: readonly Command[] = [];

/** How long "copied" stays in the status line. */
const COPIED_NOTICE_MS = 1500;

/** How long a second Ctrl-C quits. */
const QUIT_WINDOW_MS = 2000;

class Composer {
  private services = $state.raw<Services>();
  /** The draft in the input. Kept while a question has the input's place. */
  text = $state("");
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
    return () => (this.services = undefined);
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

  /** Enter: the highlighted command when the menu is open, else the draft. */
  submit(): void {
    if (this.menuVisible) {
      const command = this.commands[Math.min(this.selected, this.commands.length - 1)];
      if (command) this.choose(command);
      return;
    }
    const text = this.text.trim();
    if (text === "") return;
    this.text = "";
    const command = text.startsWith("/") ? lookup(text, this.menuEntries) : undefined;
    if (command?.run) command.run();
    else this.send(text);
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
    else this.send(`/${command.name}`);
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

  private send(text: string): void {
    const session = this.services?.daemon.sessionId;
    if (this.history.push(text) && session !== undefined) void appendHistory(session, text);
    void this.services?.daemon.send(text);
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

  /** Settles `engaged` once the pause in typing is over; returns how long to wait, if more. */
  syncEngaged(): number | undefined {
    const engaged = promptEngaged({
      hasPrompt: this.prompt !== undefined,
      engaged: this.engaged,
      now: Date.now(),
      typedAt: this.typedAt,
    });
    if (engaged !== this.engaged) this.engaged = engaged;
    if (this.prompt === undefined || engaged) return undefined;
    return Math.max(0, TYPING_PAUSE_MS - (Date.now() - this.typedAt));
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

  /** Brings back the questions put aside. */
  reopenPrompts(): void {
    this.dismissed = new Set();
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

// The questions the agent asks (a permission or a choice) arrive as chat items with buttons. The
// window answers them in a panel in place of the input: which item that is, what it shows, and
// when it takes over. Plain functions, so they are tested without a window.
import type { Block, Component, Span, ViewItem } from "@neoworks-dev/neo-protocol/types";

/** How long the user must not have typed before a question takes the input's place. */
export const TYPING_PAUSE_MS = 800;

export interface PromptButton {
  label: string;
  action: string;
  args?: unknown;
  /** The key the daemon suggests (its 1-based position). */
  key?: string;
  /** The value in effect now: the panel starts here. */
  current: boolean;
  /** What a choice says about this option, shown while it is selected. */
  description?: string;
  /** The answer is the agent's to hear: a note for it may go with it (the daemon marks it). */
  note?: boolean;
  /** Unix ms when the daemon picks it unless another is pressed: shown counting down. */
  deadline?: number;
  /** A menu that stays open (`/settings`, `/plugin`): pressed with `stay`, it is drawn again. */
  stays?: boolean;
  /** Pressing it asks for text first, which goes with the answer as `value`. */
  input?: { value?: string; multiline?: boolean };
}

export interface Prompt {
  /** The chat item, for `neo.action` and for `ui.panelRef`. */
  ref: string;
  title: string;
  /** Everything but the buttons: details, a diff, text. */
  body: Component[];
  buttons: PromptButton[];
}

/** The text of a run of blocks, paragraphs and list items on lines of their own. */
export function plainText(blocks: readonly Block[]): string {
  const spans = (list: readonly Span[]): string => list.map((span) => span.text).join("");
  const lines: string[] = [];
  for (const block of blocks) {
    switch (block.type) {
      case "paragraph":
      case "heading":
        lines.push(spans(block.spans));
        break;
      case "code_block":
        lines.push(block.code);
        break;
      case "list":
        for (const item of block.items) lines.push(`- ${plainText(item.blocks)}`);
        break;
      case "quote":
        lines.push(plainText(block.blocks));
        break;
      case "table_block":
        for (const row of [block.header, ...block.rows]) {
          lines.push(row.map((cell) => spans(cell.spans)).join(" | "));
        }
        break;
      case "rule":
        break;
    }
  }
  return lines.join("\n");
}

function containsButton(component: Component): boolean {
  switch (component.type) {
    case "button":
      return true;
    case "card":
    case "row":
    case "stack":
      return component.children.some(containsButton);
    default:
      return false;
  }
}

/**
 * Splits an item's component into its buttons (in drawing order) and the rest. In a row, text
 * after a button is that button's description (a choice's option text).
 */
export function splitPrompt(root: Component): { body: Component[]; buttons: PromptButton[] } {
  const body: Component[] = [];
  const buttons: PromptButton[] = [];

  function walk(component: Component, inRow: boolean): void {
    if (component.type === "button") {
      const { label, action, args, key, current, note, deadline, stays, input } = component;
      const button: PromptButton = { label, action, args, key, current: current === true };
      if (note === true) button.note = true;
      if (deadline !== undefined) button.deadline = deadline;
      if (stays === true) button.stays = true;
      if (input !== undefined) button.input = input;
      buttons.push(button);
      return;
    }
    if (!containsButton(component)) {
      const last = buttons.at(-1);
      if (
        inRow &&
        last !== undefined &&
        component.type === "text" &&
        last.description === undefined
      ) {
        last.description = plainText(component.blocks);
      } else {
        body.push(component);
      }
      return;
    }
    if (component.type === "card" || component.type === "row" || component.type === "stack") {
      for (const child of component.children) walk(child, component.type === "row");
    }
  }

  walk(root, false);
  return { body, buttons };
}

/** The item as a prompt, or undefined when it has no buttons. */
export function promptOf(item: ViewItem): Prompt | undefined {
  const { body, buttons } = splitPrompt(item.component);
  if (buttons.length === 0) return undefined;
  return { ref: item.ref, title: item.label ?? "", body, buttons };
}

/** The prompts still waiting for an answer, oldest first. */
export function openPrompts(items: readonly ViewItem[]): Prompt[] {
  const prompts: Prompt[] = [];
  for (const item of items) {
    if (item.state !== "pending") continue;
    const prompt = promptOf(item);
    if (prompt) prompts.push(prompt);
  }
  return prompts;
}

/** The prompt the keys answer: the oldest open one the user has not dismissed with Esc. */
export function activePrompt(
  items: readonly ViewItem[],
  dismissed: ReadonlySet<string>,
): Prompt | undefined {
  return openPrompts(items).find((prompt) => !dismissed.has(prompt.ref));
}

/** The button a prompt starts on: the one in effect now, else the first. */
export function startButton(buttons: readonly PromptButton[]): number {
  const current = buttons.findIndex((button) => button.current);
  return current < 0 ? 0 : current;
}

/** The number keys stop at 9: a choice with more options than that gets a filter field. */
export const FILTER_FROM = 9;

/** Whether the buttons are a choice long enough to filter. Permission cards never are. */
export function isLongChoice(buttons: readonly PromptButton[]): boolean {
  return buttons.length > FILTER_FROM && buttons.every((b) => b.action === "neo.choose");
}

/** The option's id, as the daemon knows it (`openrouter/anthropic/claude-haiku-4.5`). */
function optionId(button: PromptButton): string {
  const args = button.args;
  if (typeof args !== "object" || args === null || !("option" in args)) return "";
  const { option } = args;
  if (typeof option === "string") return option;
  return "";
}

/**
 * The positions of the buttons that match the filter, in their order: every word must occur, in
 * any case, in the label, the option id or the description.
 */
export function matchingButtons(buttons: readonly PromptButton[], filter: string): number[] {
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  const matches: number[] = [];
  buttons.forEach((button, index) => {
    const text = [button.label, optionId(button), button.description]
      .filter((part) => part !== undefined)
      .join("\n")
      .toLowerCase();
    if (words.every((word) => text.includes(word))) matches.push(index);
  });
  return matches;
}

/**
 * The focus after moving `delta` places among the matches, wrapping at the ends. Focus on
 * something that is no match goes to the end the move heads for.
 */
export function moveFocus(matches: readonly number[], focused: number, delta: number): number {
  const count = matches.length;
  if (count === 0) return focused;
  const at = matches.indexOf(focused);
  if (at < 0) return (delta >= 0 ? matches[0] : matches[count - 1]) ?? focused;
  return matches[(((at + delta) % count) + count) % count] ?? focused;
}

/**
 * Where the focus goes when the filter changed: the first match, or with an empty filter the
 * option in effect now. Undefined when nothing matches (the focus stays, Enter does nothing).
 */
export function focusAfterFilter(
  buttons: readonly PromptButton[],
  filter: string,
): number | undefined {
  if (filter.trim() === "") return startButton(buttons);
  return matchingButtons(buttons, filter)[0];
}

/**
 * Where to scroll a list of equal rows to show row `row`, or undefined when it shows already.
 * `top` is the scroll offset and `viewport` the height of the window.
 */
export function scrollToRow(
  row: number,
  rowHeight: number,
  top: number,
  viewport: number,
): number | undefined {
  const start = row * rowHeight;
  if (viewport <= 0) return undefined;
  if (start < top) return start;
  if (start + rowHeight > top + viewport) return start + rowHeight - viewport;
  return undefined;
}

/**
 * Whether Tab can add a note: answers the agent hears (a permission, a review of its changes)
 * carry one, settings (/settings, /model) and other buttons do not.
 */
export function takesNote(button: PromptButton | undefined): boolean {
  if (button?.note !== true) return false;
  return typeof button.args === "object" && button.args !== null;
}

/** The text a `copy:` button puts on the clipboard; undefined for buttons that act on the daemon. */
export function copyText(button: PromptButton): string | undefined {
  const prefix = "copy:";
  if (!button.action.startsWith(prefix)) return undefined;
  return button.action.slice(prefix.length);
}

/** The button's args with the user's note (when there is one) for the agent. */
export function argsWithNote(button: PromptButton, note: string): unknown {
  const text = note.trim();
  if (text === "" || typeof button.args !== "object" || button.args === null) return button.args;
  return { ...button.args, note: text };
}

/**
 * What a press sends: the button's args, with the note typed for it, the text it asked for
 * (`value`), and `stay` for a menu that stays open.
 */
export function answerArgs(
  button: PromptButton,
  note: string | undefined,
  value: string | undefined,
): unknown {
  const args = note === undefined ? button.args : argsWithNote(button, note);
  if (typeof args !== "object" || args === null) return args;
  return {
    ...args,
    ...(value !== undefined ? { value } : {}),
    ...(button.stays === true ? { stay: true } : {}),
  };
}

/**
 * Whether the prompt has taken the input's place. It waits while the user has a message half
 * written (`drafting`), until it is sent or emptied or the user asks for the question, and for a
 * pause in typing after that (so a card appearing mid-sentence does not swallow keystrokes); once
 * shown it stays until answered or dismissed.
 */
export function promptEngaged(state: {
  hasPrompt: boolean;
  engaged: boolean;
  drafting: boolean;
  now: number;
  typedAt: number;
}): boolean {
  if (!state.hasPrompt) return false;
  if (state.engaged) return true;
  return !state.drafting && state.now - state.typedAt >= TYPING_PAUSE_MS;
}

/** The first lines of a diff, code or text body for the compact panel. */
export function previewLines(text: string, max: number): { lines: string[]; more: number } {
  const all = text.split("\n");
  return { lines: all.slice(0, max), more: Math.max(0, all.length - max) };
}

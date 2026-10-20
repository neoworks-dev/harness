// The visual state of OpenUI docs (the selected tab, an open accordion item, the slide a carousel
// starts at, the value just chosen here), kept for the window's lifetime: the chat's list builds
// a row anew each time it scrolls into view, and the state must outlive that. The daemon holds
// the rest (docs/openui.md): a doc here only draws, and sends what the user does.
import { clipboard } from "@neoworks/svelte-gpui";
import type { UiDoc, UiNode } from "@suite/protocol/types";
import { firstButton, neighbour, focusOrder } from "./focus.ts";
import { DEBOUNCE_MS, HOLD_MS, outbox, same, type Send } from "./form.ts";

/** A value chosen here that the daemon has not echoed yet. */
interface Held {
  value: unknown;
  at: number;
}

/** What one doc remembers. */
export class DocView {
  tabs = $state<Record<string, number>>({});
  open = $state<Record<string, boolean>>({});
  slides = $state<Record<string, number>>({});
  /** The item a focused checkbox or switch group has its cursor on. */
  cursors = $state<Record<string, number>>({});
  held = $state<Record<string, Held>>({});
  /** The id of what has the keyboard (a key or a path), or nothing. */
  focus = $state<string>();
  /** What can be focused now, by id: set while it is drawn. */
  private handles = new Map<string, () => void>();

  tab(path: string): number {
    return this.tabs[path] ?? 0;
  }

  isOpen(path: string, starts: boolean): boolean {
    return this.open[path] ?? starts;
  }

  /** Remembers a value chosen here until the daemon echoes it (or a moment has passed). */
  hold(key: string, value: unknown): void {
    const at = Date.now();
    this.held[key] = { value, at };
    setTimeout(() => {
      if (this.held[key]?.at === at) delete this.held[key];
    }, HOLD_MS);
  }

  /** The daemon sent `value` for a field: a held value it has caught up with is not needed. */
  echoed(key: string, value: unknown): void {
    const held = this.held[key];
    if (held !== undefined && same(held.value, value)) delete this.held[key];
  }

  /** Registers what takes the keyboard under `id`; returns the function that takes it away. */
  register(id: string, take: () => void): () => void {
    this.handles.set(id, take);
    return () => {
      if (this.handles.get(id) === take) this.handles.delete(id);
    };
  }

  /** Gives the keyboard to the id; false when it is not drawn. */
  take(id: string): boolean {
    const take = this.handles.get(id);
    if (take === undefined) return false;
    take();
    return true;
  }

  /** Tab and Shift-Tab: the next thing in drawing order that is drawn. */
  move(root: UiNode | undefined, by: 1 | -1): boolean {
    if (root === undefined) return false;
    const next = neighbour(focusOrder(root, this), this.focus, by, (id) => this.handles.has(id));
    return next !== undefined && this.take(next);
  }
}

const views = new Map<string, DocView>();

/** The state of the doc `id` in chat item `ref` of a session. */
export function viewOf(session: string | undefined, ref: string, id: string): DocView {
  const key = JSON.stringify([session, ref, id]);
  let view = views.get(key);
  if (view === undefined) {
    view = new DocView();
    views.set(key, view);
  }
  return view;
}

/** What a doc needs of the window around it. */
export interface Host {
  ref: string;
  ui: UiDoc;
  view: DocView;
  send: Send;
  /** Opens an address in the system's browser (or says it can't). */
  open: (url: string) => void;
}

/** One doc as its parts reach it: the daemon's values, what was chosen here, what is sent. */
export class DocSession {
  constructor(private host: () => Host) {}

  get ref(): string {
    return this.host().ref;
  }
  get doc(): string {
    return this.host().ui.id;
  }
  get view(): DocView {
    return this.host().view;
  }
  /** The doc's root, for moving between its fields. */
  get rootNode(): UiNode | undefined {
    return this.host().ui.root;
  }
  /** Still arriving: it draws, but no field or button can be used. */
  get streaming(): boolean {
    return this.host().ui.streaming === true;
  }

  /** The value of a field now: what was chosen here, else the daemon's. */
  value(node: UiNode): unknown {
    const key = node.key;
    const held = key === undefined ? undefined : this.view.held[key];
    return held === undefined ? node.props["value"] : held.value;
  }

  /**
   * A field's new value. It goes at once, or after a pause with `soon` (typing, a slider being
   * dragged). Until the daemon echoes it the value shows as chosen, unless `hold` is false: a text
   * field keeps its own text while it has focus.
   */
  input(node: UiNode, value: unknown, options: { soon?: boolean; hold?: boolean } = {}): void {
    const key = node.key;
    if (key === undefined || this.streaming) return;
    if (options.hold !== false) this.view.hold(key, value);
    const delay = options.soon === true ? DEBOUNCE_MS : 0;
    outbox.queue(this.ref, { doc: this.doc, key, value }, delay, this.host().send);
  }

  /** The button Enter presses in a field: the form's first, else the doc's first one that can be pressed. */
  firstButton(form?: UiNode): UiNode | undefined {
    return firstButton(form ?? this.host().ui.root);
  }

  /** Opens an address (a link). */
  open(url: string): void {
    this.host().open(url);
  }

  /** What waits for a field goes out now (it lost the keyboard). */
  flush(node: UiNode): void {
    if (node.key !== undefined) outbox.flush(this.ref, this.doc, node.key);
  }

  /** Presses a button: the form's values first, then the press; an address is opened, text copied. */
  press(node: UiNode): void {
    if (this.streaming || node.props["disabled"] === true) return;
    const url = node.props["url"];
    const copy = node.props["copy"];
    if (typeof url === "string") this.host().open(url);
    else if (typeof copy === "string") void clipboard.write(copy);
    else if (node.key !== undefined) outbox.press(this.ref, this.doc, node.key, this.host().send);
  }
}

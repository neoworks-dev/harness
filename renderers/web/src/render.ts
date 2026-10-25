// mountChat: draws a session's view into a container and keeps it current. The DOM is updated per
// item: a `set` replaces that item's element, `remove` drops it, and a streaming `blocks` op
// re-renders only the tail of the text component it targets.
import type {
  ButtonComponent,
  Theme,
  ViewItem,
  ViewPatch,
  ViewSnapshot,
} from "@neoworks-dev/neo-protocol/types";
import { componentAtPath, ViewState, type ViewChange } from "@neoworks-dev/neo-sdk";
import { buttonKey, renderBlocks, renderItem } from "./components.ts";
import { CHAT_CLASS, chatStyles, themeVariables } from "./styles.ts";

/** What mountChat needs of `navigator.clipboard`. */
export interface ClipboardLike {
  writeText(text: string): Promise<void>;
}

export interface ChatOptions {
  /** Applied right away; the daemon sends it after connecting, so it may come later (setTheme). */
  theme?: Theme;
  /**
   * A button with a non-`copy:` action was pressed in item `ref`: send `ui.action`, e.g.
   * `client.action(session, ref, action, args)`.
   */
  onAction?: (ref: string, action: string, args?: unknown) => void;
  /** Where `copy:` actions write to; defaults to the page's `navigator.clipboard`. */
  clipboard?: ClipboardLike;
  /** Copying failed (no clipboard, permission denied). */
  onError?: (error: Error) => void;
  /**
   * Maps an image `src` to something the page can load, e.g. `asset://img-0123456789ab` to
   * `${daemonUrl}/assets/img-0123456789ab?token=…`. Return undefined to leave `src` alone. Only
   * `http(s):` and `data:image/` results are used; anything else is ignored. Images already
   * drawn are not redrawn when the resolver changes what it returns.
   */
  resolveImage?: (src: string) => string | undefined;
  /** Insert the stylesheet into the document's head once (default true). */
  injectStyles?: boolean;
}

export interface Chat {
  /** Replaces the whole list, keeping the DOM of items that did not change. */
  setItems(items: readonly ViewItem[]): void;
  /** Same as setItems, for a `view.snapshot` (remembers its session and seq for patches). */
  applySnapshot(snapshot: ViewSnapshot): void;
  /** Applies a `view.patch`, touching only the DOM of the items it changes. */
  applyPatch(patch: ViewPatch): void;
  setTheme(theme: Theme): void;
  /** The items as currently drawn. */
  readonly items: readonly ViewItem[];
  /** Detaches listeners and timers; the rendered DOM stays. */
  destroy(): void;
}

interface Entry {
  item: ViewItem;
  el: HTMLElement;
  body: HTMLElement;
  buttons: ButtonComponent[];
}

const STYLE_ATTRIBUTE = "data-neo-chat-styles";
const COPIED_MS = 1500;
const ARROWS: Record<string, number> = { ArrowDown: 1, ArrowUp: -1 };

function isElement(target: EventTarget | null): target is Element {
  return target !== null && "closest" in target && typeof target.closest === "function";
}

/** The element of the component at `path` below `body` (child elements, minus a card's title). */
function componentAt(body: Element, path: readonly number[]): Element | undefined {
  let node = body.firstElementChild ?? undefined;
  for (const index of path) {
    if (!node) return undefined;
    const children = Array.from(node.children).filter(
      (child) => !child.classList.contains("neo-card-title"),
    );
    node = children[index];
  }
  return node;
}

function injectStyles(doc: Document): void {
  if (doc.head.querySelector(`style[${STYLE_ATTRIBUTE}]`)) return;
  const style = doc.createElement("style");
  style.setAttribute(STYLE_ATTRIBUTE, "");
  style.textContent = chatStyles;
  doc.head.append(style);
}

function asError(err: unknown): Error {
  if (err instanceof Error) return err;
  return new Error(String(err));
}

export function mountChat(container: HTMLElement, options: ChatOptions = {}): Chat {
  const doc = container.ownerDocument;
  const state = new ViewState();
  const entries = new Map<string, Entry>();
  const timers = new Set<ReturnType<typeof setTimeout>>();

  container.classList.add(CHAT_CLASS);
  container.setAttribute("role", "log");
  if (options.injectStyles !== false) injectStyles(doc);

  function build(item: ViewItem): Entry {
    const { el, body, buttons } = renderItem(doc, item, options.resolveImage);
    return { item, el, body, buttons };
  }

  /** Keeps the view at the bottom if it was there before `change` made the content grow. */
  function pinned(change: () => void): void {
    const atBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 48;
    change();
    if (atBottom) container.scrollTop = container.scrollHeight;
  }

  /** A fresh element for `item` in place of `entry`'s, keeping focus and a thought's open state. */
  function replace(entry: Entry, item: ViewItem): Entry {
    const next = build(item);
    const active = doc.activeElement;
    const focused = active !== null && entry.el.contains(active);
    const button = active?.getAttribute("data-button");
    if (entry.el.hasAttribute("open")) next.el.setAttribute("open", "");
    entry.el.replaceWith(next.el);
    if (focused) {
      const target = next.el.querySelector<HTMLElement>(`[data-button="${button}"]`);
      (target ?? next.el).focus();
    }
    return next;
  }

  function sync(): void {
    const wanted = new Set(state.items.map((item) => item.ref));
    for (const [ref, entry] of entries) {
      if (wanted.has(ref)) continue;
      entry.el.remove();
      entries.delete(ref);
    }
    let cursor: Element | null = container.firstElementChild;
    for (const item of state.items) {
      let entry = entries.get(item.ref);
      if (!entry) entry = build(item);
      else if (entry.item !== item && JSON.stringify(entry.item) !== JSON.stringify(item)) {
        entry = replace(entry, item);
      } else entry.item = item;
      entries.set(item.ref, entry);
      if (entry.el === cursor) cursor = cursor.nextElementSibling;
      else container.insertBefore(entry.el, cursor);
    }
  }

  function applyChange(change: ViewChange): void {
    switch (change.type) {
      case "added": {
        const entry = build(change.item);
        entries.set(change.item.ref, entry);
        const following = state.items[change.index + 1];
        const before = following ? entries.get(following.ref) : undefined;
        container.insertBefore(entry.el, before?.el ?? null);
        return;
      }
      case "replaced": {
        const entry = entries.get(change.item.ref);
        if (entry) entries.set(change.item.ref, replace(entry, change.item));
        return;
      }
      case "removed": {
        entries.get(change.ref)?.el.remove();
        entries.delete(change.ref);
        return;
      }
      case "blocks": {
        const entry = entries.get(change.item.ref);
        if (!entry) return;
        if (rerenderTail(entry, change)) entry.item = change.item;
        else entries.set(change.item.ref, replace(entry, change.item));
      }
    }
  }

  /** Streaming text: drop the DOM of blocks[from:] and draw the new tail. False: redraw the item. */
  function rerenderTail(entry: Entry, change: Extract<ViewChange, { type: "blocks" }>): boolean {
    const host = componentAt(entry.body, change.path);
    const component = componentAtPath(change.item.component, change.path);
    if (!host?.classList.contains("neo-text") || component?.type !== "text") return false;
    while (host.children.length > change.from) host.lastElementChild?.remove();
    const tail = component.blocks.slice(host.children.length);
    host.append(...renderBlocks({ doc, buttons: [] }, tail));
    return host.children.length === component.blocks.length;
  }

  // --- buttons and keys ---

  function itemOf(target: EventTarget | null): Entry | undefined {
    if (!isElement(target)) return undefined;
    const ref = target.closest("[data-ref]")?.getAttribute("data-ref");
    if (ref === null || ref === undefined) return undefined;
    return entries.get(ref);
  }

  function copy(button: HTMLElement | undefined, text: string): void {
    const clipboard = options.clipboard ?? doc.defaultView?.navigator.clipboard;
    if (!clipboard) {
      options.onError?.(new Error("no clipboard available"));
      return;
    }
    clipboard.writeText(text).then(
      () => {
        if (!button) return;
        button.setAttribute("data-copied", "");
        const timer = setTimeout(() => {
          timers.delete(timer);
          button.removeAttribute("data-copied");
        }, COPIED_MS);
        timers.add(timer);
      },
      (err: unknown) => options.onError?.(asError(err)),
    );
  }

  function activate(entry: Entry, index: number, button?: HTMLElement): void {
    const target = entry.buttons[index];
    if (!target) return;
    if (target.action.startsWith("copy:")) copy(button, target.action.slice("copy:".length));
    else options.onAction?.(entry.item.ref, target.action, target.args);
  }

  /** Items with buttons, in order: the ones Tab and the arrow keys stop at. */
  function focusable(): Entry[] {
    const list: Entry[] = [];
    for (const item of state.items) {
      const entry = entries.get(item.ref);
      if (entry && entry.buttons.length > 0) list.push(entry);
    }
    return list;
  }

  function onClick(event: Event): void {
    if (!isElement(event.target)) return;
    const button = event.target.closest<HTMLElement>("[data-button]");
    const entry = itemOf(button);
    if (!button || !entry) return;
    activate(entry, Number(button.getAttribute("data-button")), button);
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return;
    const entry = itemOf(event.target);
    const step = ARROWS[event.key];
    if (step !== undefined) {
      const list = focusable();
      const current = entry ? list.indexOf(entry) : -1;
      let next = current + step;
      if (current < 0 && step < 0) next = list.length - 1;
      next = Math.min(Math.max(next, 0), list.length - 1);
      const target = list[next];
      if (!target) return;
      event.preventDefault();
      target.el.focus();
      return;
    }
    if (!entry) return;
    // A key press on a button itself is that button's own business (Enter, Space).
    const index = entry.buttons.findIndex((button, i) => buttonKey(button, i) === event.key);
    if (index < 0) return;
    event.preventDefault();
    activate(
      entry,
      index,
      entry.el.querySelector<HTMLElement>(`[data-button="${index}"]`) ?? undefined,
    );
  }

  container.addEventListener("click", onClick);
  container.addEventListener("keydown", onKeyDown);

  function setTheme(theme: Theme): void {
    for (const [name, value] of Object.entries(themeVariables(theme))) {
      container.style.setProperty(name, value);
    }
  }
  if (options.theme) setTheme(options.theme);

  return {
    setItems(items) {
      pinned(() => {
        state.reset([...items]);
        sync();
      });
    },
    applySnapshot(snapshot) {
      pinned(() => {
        state.applySnapshot(snapshot);
        sync();
      });
    },
    applyPatch(patch) {
      pinned(() => {
        for (const change of state.applyPatch(patch)) applyChange(change);
      });
    },
    setTheme,
    get items() {
      return state.items;
    },
    destroy() {
      container.removeEventListener("click", onClick);
      container.removeEventListener("keydown", onKeyDown);
      for (const timer of timers) clearTimeout(timer);
      timers.clear();
    },
  };
}

// The browser page's behavior, kept apart from the page shell so tests can run it under happy-dom:
// the address bar, the page's text and images (with a Download button each), and the agent's chat
// pane, which always shows the daemon's current session and lights up when the agent is here.
import { mountChat, type Chat } from "@suite/renderer-web";
import type { BusClient } from "@suite/sdk/bus";
import type { DownloadView, PageView } from "../src/types.ts";
import type { PageApi } from "./api.ts";

export interface PageElements {
  addressForm: HTMLFormElement;
  addressInput: HTMLInputElement;
  title: HTMLElement;
  /** Loading state, errors and counts. */
  note: HTMLElement;
  /** Grid of the page's images. */
  images: HTMLElement;
  text: HTMLElement;
  /** The chat pane as a whole: gets the `focused` class while the agent is in this app. */
  pane: HTMLElement;
  chat: HTMLElement;
  chatForm: HTMLFormElement;
  chatInput: HTMLInputElement;
  status: HTMLElement;
}

export interface PageOptions {
  /** See ChatOptions.resolveImage: where to load `asset://` images from. */
  resolveImage?: (src: string) => string | undefined;
}

export interface BrowserPage {
  readonly chat: Chat;
  /** The session the pane shows, once the first agent.focus arrived. */
  readonly session: string | undefined;
  /** Same as typing into the address bar and pressing Enter. */
  go(input: string): Promise<void>;
  /** Shows a page (null: the empty state). */
  show(page: PageView | null): void;
  stop(): void;
}

const APP = "browser";

function message(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

function isHttp(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

/** `client` may be undefined (no daemon): the address bar and the page work, the chat doesn't. */
export function startBrowserPage(
  client: BusClient | undefined,
  api: PageApi,
  els: PageElements,
  options: PageOptions = {},
): BrowserPage {
  const doc = els.images.ownerDocument;
  let session: string | undefined;
  let view: { close(): void } | undefined;
  /** Tile handle labels by image URL, so a download made by the agent shows up too. */
  const handles = new Map<string, HTMLElement>();
  let shown = 0;

  const report = (text: string): void => {
    els.status.textContent = text;
  };

  // --- the page ---

  function tile(image: { src: string; alt: string }): HTMLElement {
    const box = doc.createElement("figure");
    box.className = "tile";
    box.style.margin = "0";
    const img = doc.createElement("img");
    img.setAttribute("src", image.src);
    img.setAttribute("alt", image.alt);
    img.setAttribute("loading", "lazy");
    img.setAttribute("referrerpolicy", "no-referrer");
    const alt = doc.createElement("div");
    alt.className = "alt";
    alt.textContent = image.alt;
    const handle = doc.createElement("div");
    handle.className = "handle";
    const button = doc.createElement("button");
    button.type = "button";
    button.textContent = "Download";
    button.addEventListener("click", () => {
      button.disabled = true;
      handle.textContent = "downloading…";
      api.download(image.src).then(
        (done) => {
          button.disabled = false;
          handle.textContent = done.handle;
        },
        (err: unknown) => {
          button.disabled = false;
          handle.textContent = message(err);
        },
      );
    });
    handles.set(image.src, handle);
    box.append(img, alt, button, handle);
    return box;
  }

  function show(page: PageView | null): void {
    shown += 1;
    handles.clear();
    if (page === null) {
      els.title.textContent = "";
      els.text.textContent = "";
      els.images.replaceChildren();
      els.note.textContent = "";
      return;
    }
    els.title.textContent = page.title === "" ? page.url : page.title;
    els.addressInput.value = page.url;
    els.note.textContent = `${page.images.length} images · ${page.links} links`;
    els.text.textContent = page.text;
    els.images.replaceChildren(...page.images.filter((i) => isHttp(i.src)).map(tile));
  }

  async function go(input: string): Promise<void> {
    const text = input.trim();
    if (text === "") return;
    els.note.textContent = `loading ${text}…`;
    try {
      show(await api.navigate(text));
    } catch (err) {
      els.note.textContent = message(err);
    }
  }

  const onAddress = (event: Event): void => {
    event.preventDefault();
    void go(els.addressInput.value);
  };
  els.addressForm.addEventListener("submit", onAddress);

  const offEvents = api.onEvent((event) => {
    if (event.type === "page") show(event.page);
    else {
      const done: DownloadView = event.download;
      const label = handles.get(done.url);
      if (label) label.textContent = done.handle;
      report(`downloaded ${done.name} as ${done.handle}`);
    }
  });

  // --- the agent's chat ---

  const chat = mountChat(els.chat, {
    resolveImage: options.resolveImage,
    onAction: (ref, action, args) => {
      if (session === undefined || !client) return;
      client.action(session, ref, action, args).catch((err: unknown) => report(message(err)));
    },
    onError: (err) => report(err.message),
  });
  const offs: (() => void)[] = [];
  const onChat = (event: Event): void => {
    event.preventDefault();
    const text = els.chatInput.value.trim();
    if (text === "" || session === undefined || !client) return;
    els.chatInput.value = "";
    client.input(session, text, { app: APP }).catch((err: unknown) => report(message(err)));
  };
  els.chatForm.addEventListener("submit", onChat);

  if (client) {
    const bus = client;
    const follow = (next: string): void => {
      if (next === session) return;
      view?.close();
      session = next;
      chat.setItems([]);
      view = bus.subscribeView(next, {
        onSnapshot: (snapshot) => chat.applySnapshot(snapshot),
        onPatch: (patch) => chat.applyPatch(patch),
        onError: (err) => report(err.message),
      });
      report(`session ${next}`);
    };
    const focusChanged = (focus: { session: string; app?: string }): void => {
      follow(focus.session);
      els.pane.classList.toggle("focused", focus.app === APP);
    };
    if (bus.theme) chat.setTheme(bus.theme);
    offs.push(
      bus.onTheme((theme) => chat.setTheme(theme)),
      bus.onFocus(focusChanged),
      bus.onConnectionChange((connected) => {
        if (connected) report(`session ${session ?? "?"}`);
        else report("disconnected, reconnecting…");
      }),
    );
    if (bus.focus) focusChanged(bus.focus);
  }

  return {
    chat,
    get session() {
      return session;
    },
    go,
    show,
    stop() {
      offEvents();
      for (const off of offs) off();
      view?.close();
      els.addressForm.removeEventListener("submit", onAddress);
      els.chatForm.removeEventListener("submit", onChat);
      chat.destroy();
    },
  };
}

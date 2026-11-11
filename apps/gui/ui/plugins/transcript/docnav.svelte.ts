// Ctrl-P takes the keyboard into the chat's latest neoui doc with something to press or fill in,
// as the terminal does: the chat scrolls to it and its first part has the keyboard, the arrows and
// Tab move through it. Ctrl-P again goes to an older doc, Ctrl-N to a newer one and past the
// newest back to the input; Esc leaves it too.
import { interactiveDocs, sameDoc, type DocTarget } from "./docs.ts";
import { onDocBlur, viewOf, type DocView } from "./neoui/state.svelte.ts";
import type { Row } from "./rows.ts";

/** How long a doc scrolled to may take to be built before giving up on it. */
const BUILD_TRIES = 30;

export interface Chat {
  session(): string | undefined;
  rows(): readonly Row[];
  /** Scrolls the row into view. */
  reveal(index: number): void;
}

class DocNav {
  /** The doc the keyboard was taken into, while it is there. */
  active = $state.raw<DocTarget>();
  private chat?: Chat;

  /** The chat the docs are in, while it is drawn; returns the call that lets it go. */
  attach(chat: Chat): () => void {
    this.chat = chat;
    const stop = onDocBlur((view) => this.blurred(view));
    return () => {
      stop();
      if (this.chat === chat) this.chat = undefined;
      this.active = undefined;
    };
  }

  /** Ctrl-P (-1) or Ctrl-N (1): the next doc that way. False when there was none to go to. */
  step(by: 1 | -1): boolean {
    const chat = this.chat;
    if (chat === undefined) return false;
    const docs = interactiveDocs(chat.rows());
    const active = this.active;
    const at = active === undefined ? -1 : docs.findIndex((doc) => sameDoc(doc, active));
    if (at < 0) {
      // Not in one: Ctrl-P starts at the newest, Ctrl-N is someone else's.
      const newest = docs.at(-1);
      if (by > 0 || newest === undefined) return false;
      this.enter(chat, newest);
      return true;
    }
    if (at + by >= docs.length) this.leave();
    else this.enter(chat, docs[Math.max(0, at + by)]!);
    return true;
  }

  leave(): void {
    this.active = undefined;
  }

  private view(chat: Chat, target: DocTarget): DocView {
    return viewOf(chat.session(), target.ref, target.ui.id);
  }

  private enter(chat: Chat, target: DocTarget): void {
    this.active = target;
    chat.reveal(target.row);
    const view = this.view(chat, target);
    // The list builds a row once it is scrolled to: wait for its parts to be drawn.
    let tries = 0;
    const attempt = (): void => {
      if (this.active !== target) return;
      if (view.first(target.ui.root)) return;
      if (++tries < BUILD_TRIES) setTimeout(attempt, 16);
      else this.leave();
    };
    attempt();
  }

  /** The keyboard left a part of a doc: unless it went on to another part of it (Esc, a click elsewhere), the doc is left. */
  private blurred(view: DocView): void {
    setTimeout(() => {
      const chat = this.chat;
      const active = this.active;
      if (chat === undefined || active === undefined) return;
      if (this.view(chat, active) === view && view.focus === undefined) this.leave();
    }, 0);
  }
}

export const docNav = new DocNav();

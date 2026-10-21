// The demo page's behavior, kept apart from the page shell so tests can run it under happy-dom:
// follow the daemon's current session (agent.focus), draw its view, send typed input and button
// presses back.
import type { AgentFocus, Theme } from "@neoworks-dev/neo-protocol/types";
import type { Client, ConnectionState, Unsubscribe } from "@neoworks-dev/neo-sdk";
import { mountChat, type Chat } from "../src/render.ts";

export interface DemoElements {
  chat: HTMLElement;
  /** Shows the connection state and errors. */
  status: HTMLElement;
  form: HTMLFormElement;
  input: HTMLInputElement;
}

export interface DemoOptions {
  /** See ChatOptions.resolveImage: where to load `asset://` images from. */
  resolveImage?: (src: string) => string | undefined;
}

export interface Demo {
  readonly chat: Chat;
  /** The session being followed, once the first agent.focus arrived. */
  readonly session: string | undefined;
  /** Sends `text` as input (app "web") to the followed session. */
  send(text: string): Promise<void>;
  stop(): void;
}

function message(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

export function startDemo(client: Client, els: DemoElements, options: DemoOptions = {}): Demo {
  let session: string | undefined;
  let unfollowView: Unsubscribe | undefined;

  function report(text: string): void {
    els.status.textContent = text;
  }

  const chat = mountChat(els.chat, {
    resolveImage: options.resolveImage,
    onAction: (ref, action, args) => {
      if (session === undefined) return;
      client.action(session, ref, action, args).catch((err: unknown) => report(message(err)));
    },
    onError: (err) => report(err.message),
  });

  /** Subscribes to `next`'s view; the daemon answers with a snapshot, which replaces the list. */
  function follow(next: string): void {
    if (next === session) return;
    unfollowView?.();
    session = next;
    chat.setItems([]);
    unfollowView = client.subscribeView(next, {
      onSnapshot: (snapshot) => chat.applySnapshot(snapshot),
      onPatch: (patch) => chat.applyPatch(patch),
      onError: (err) => report(err.message),
    });
    report(`session ${next}`);
  }

  async function send(text: string): Promise<void> {
    if (session === undefined) throw new Error("no session yet");
    await client.input(session, text, { app: "web" });
  }

  const onSubmit = (event: Event): void => {
    event.preventDefault();
    const text = els.input.value.trim();
    if (text === "") return;
    els.input.value = "";
    send(text).catch((err: unknown) => report(message(err)));
  };
  els.form.addEventListener("submit", onSubmit);

  const themeChanged = (theme: Theme | undefined): void => {
    if (theme) chat.setTheme(theme);
  };
  const focusChanged = (focus: AgentFocus | undefined): void => {
    if (focus) follow(focus.session);
  };
  const stateChanged = (state: ConnectionState): void => {
    if (state === "open") report(`session ${session ?? "?"}`);
    else if (state === "reconnecting") report("disconnected, reconnecting…");
    else if (state === "closed") report("disconnected");
  };
  themeChanged(client.theme.value);
  const stops = [
    client.theme.on(themeChanged),
    client.focus.on(focusChanged),
    client.state.on(stateChanged),
  ];
  focusChanged(client.focus.value);

  return {
    chat,
    get session() {
      return session;
    },
    send,
    stop() {
      for (const stop of stops) stop();
      unfollowView?.();
      els.form.removeEventListener("submit", onSubmit);
      chat.destroy();
    },
  };
}

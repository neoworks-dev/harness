// The `daemon` service: the window's connection to the daemon and the session it shows, from the
// `neo` store (kernel/neo.svelte.ts). Everything it reads is reactive. No `$state` of its own:
// the store has it (see the README).
import { Service, type Context } from "@neoworks/extension-system";
import type { SecretRequest, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { neo } from "../neo.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    daemon: DaemonService;
  }
}

type Store = typeof neo;

export class DaemonService extends Service {
  constructor(ctx: Context) {
    super(ctx, "daemon");
  }

  /** Connects to the daemon and opens the session `neo gui` asked for. */
  start(): Promise<void> {
    return neo.start();
  }

  get connection(): Store["connection"] {
    return neo.connection;
  }
  /** Why starting or the last request failed. */
  get error(): Store["error"] {
    return neo.error;
  }
  set error(message: Store["error"]) {
    neo.error = message;
  }
  /** What the window draws with. */
  get palette(): Store["palette"] {
    return neo.palette;
  }
  /** Every session, kept current by the daemon. */
  get sessions(): Store["sessions"] {
    return neo.sessions;
  }
  /** The name this window registered under: desktop, desktop-2… */
  get appId(): Store["appId"] {
    return neo.appId;
  }
  /** The session this window shows, and its directory. */
  get sessionId(): Store["sessionId"] {
    return neo.sessionId;
  }
  get session(): Store["session"] {
    return neo.session;
  }
  get cwd(): Store["cwd"] {
    return neo.cwd;
  }
  /** The shown session's chat, as the daemon resolved it. */
  get items(): Store["items"] {
    return neo.items;
  }
  /** The shown session's open surfaces (neoui drawn in a bar, a panel or a page). */
  get surfaces(): Store["surfaces"] {
    return neo.surfaces;
  }
  get status(): Store["status"] {
    return neo.status;
  }
  get branch(): Store["branch"] {
    return neo.branch;
  }

  /** Shows a session in this window and moves the agent here with it. */
  open(id: string): Promise<void> {
    return neo.open(id);
  }
  /** Starts a session (in `cwd`, else the shown one's directory) and shows it. */
  newSession(cwd?: string): Promise<void> {
    return neo.newSession(cwd);
  }
  /** Sends a command to a session that may not be the shown one, such as `/title` from the sidebar. */
  sendTo(session: string, text: string): Promise<void> {
    return neo.sendTo(session, text);
  }
  /**
   * Sends what the user typed, commands included, with the paths of attached images; `steer`
   * into the running turn at once.
   */
  send(text: string, images?: string[], steer = false): Promise<void> {
    return neo.send(text, images, steer);
  }
  /** Takes the user's messages queued behind the running turn back, oldest first. */
  unqueue(): ReturnType<typeof neo.unqueue> {
    return neo.unqueue();
  }
  /**
   * What each harness the session can switch to offers. Asking may start a guest's adapter,
   * unless `known`: then it starts nothing and leaves out what is not known yet.
   */
  settings(known = false): ReturnType<typeof neo.settings> {
    return neo.settings(known);
  }
  /** What the harnesses offered when last asked (the pickers' and the status line's names). */
  get offered(): Store["offered"] {
    return neo.offered;
  }
  /** The name a harness (by default the session's) gives a value of a setting (`Haiku 5.5`), else the value. */
  label(category: "model" | "thought_level" | "mode", value: string, harness?: string): string {
    return neo.label(category, value, harness);
  }
  cancel(): Promise<void> {
    return neo.cancel();
  }
  /** A button in chat item `ref` was pressed (`args` carries e.g. a note for the agent). */
  action(ref: string, action: string, args?: unknown): Promise<void> {
    return neo.action(ref, action, args);
  }
  /** Shift-Tab: the next approvals mode. */
  cycleApprovals(): Promise<void> {
    return neo.cycleApprovals();
  }
  /** Has the daemon download a public web image into its asset store (once per URL); the asset's handle. */
  fetchAsset(url: string): Promise<string> {
    return neo.fetchAsset(url);
  }
  /** The URL an `asset://` handle loads from, for an `Image`. It holds the daemon's token. */
  assetSrc(src: string): string | undefined {
    return neo.assetSrc(src);
  }

  /**
   * Follows one agent's view (a subagent or a guest) in the shown session. Stops with the returned
   * call, or when the plugin unloads.
   */
  followAgent(agent: string, onItems: (items: readonly ViewItem[]) => void): () => void {
    const stop = this.ctx.effect(() => neo.followAgent(agent, onItems), "daemon.followAgent");
    return () => void stop();
  }

  /** Every event of the shown session from now on; stops when the plugin unloads. */
  onRecord(listener: Parameters<Store["onRecord"]>[0]): () => void {
    const stop = this.ctx.effect(() => neo.onRecord(listener), "daemon.onRecord");
    return () => void stop();
  }

  /** The daemon asks this window for a secret; stops when the plugin unloads. */
  onSecretRequest(listener: (request: SecretRequest) => void): () => void {
    const stop = this.ctx.effect(() => neo.onSecretRequest(listener), "daemon.onSecretRequest");
    return () => void stop();
  }

  /** Answers a secret request with the typed value, or without one when cancelled. */
  answerSecret(id: string, value?: string): void {
    neo.answerSecret(id, value);
  }
}

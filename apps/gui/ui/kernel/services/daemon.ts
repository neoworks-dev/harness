// The `daemon` service: the window's connection to the daemon and the session it shows, from the
// `suite` store (kernel/suite.svelte.ts). Everything it reads is reactive. No `$state` of its own:
// the store has it (see the README).
import { Service, type Context } from "@neoworks/extension-system";
import type { ViewItem } from "@suite/protocol/types";
import { suite } from "../suite.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    daemon: DaemonService;
  }
}

type Store = typeof suite;

export class DaemonService extends Service {
  constructor(ctx: Context) {
    super(ctx, "daemon");
  }

  /** Connects to the daemon and opens the session `suite gui` asked for. */
  start(): Promise<void> {
    return suite.start();
  }

  get connection(): Store["connection"] {
    return suite.connection;
  }
  /** Why starting or the last request failed. */
  get error(): Store["error"] {
    return suite.error;
  }
  set error(message: Store["error"]) {
    suite.error = message;
  }
  /** What the window draws with. */
  get palette(): Store["palette"] {
    return suite.palette;
  }
  /** Every session, kept current by the daemon. */
  get sessions(): Store["sessions"] {
    return suite.sessions;
  }
  /** The name this window registered under: desktop, desktop-2… */
  get appId(): Store["appId"] {
    return suite.appId;
  }
  /** The session this window shows, and its directory. */
  get sessionId(): Store["sessionId"] {
    return suite.sessionId;
  }
  get session(): Store["session"] {
    return suite.session;
  }
  get cwd(): Store["cwd"] {
    return suite.cwd;
  }
  /** The shown session's chat, as the daemon resolved it. */
  get items(): Store["items"] {
    return suite.items;
  }
  get status(): Store["status"] {
    return suite.status;
  }
  get branch(): Store["branch"] {
    return suite.branch;
  }

  /** Shows a session in this window and moves the agent here with it. */
  open(id: string): Promise<void> {
    return suite.open(id);
  }
  /** Starts a session (in `cwd`, else the shown one's directory) and shows it. */
  newSession(cwd?: string): Promise<void> {
    return suite.newSession(cwd);
  }
  /** Sends what the user typed, commands included. */
  send(text: string): Promise<void> {
    return suite.send(text);
  }
  cancel(): Promise<void> {
    return suite.cancel();
  }
  /** A button in chat item `ref` was pressed (`args` carries e.g. a note for the agent). */
  action(ref: string, action: string, args?: unknown): Promise<void> {
    return suite.action(ref, action, args);
  }
  /** Shift-Tab: the next approvals mode. */
  cycleApprovals(): Promise<void> {
    return suite.cycleApprovals();
  }
  /** The URL an `asset://` handle loads from, for an `Image`. It holds the daemon's token. */
  assetSrc(src: string): string | undefined {
    return suite.assetSrc(src);
  }

  /**
   * Follows one agent's view (a subagent or a guest) in the shown session. Stops with the returned
   * call, or when the plugin unloads.
   */
  followAgent(agent: string, onItems: (items: readonly ViewItem[]) => void): () => void {
    const stop = this.ctx.effect(() => suite.followAgent(agent, onItems), "daemon.followAgent");
    return () => void stop();
  }

  /** Every event of the shown session from now on; stops when the plugin unloads. */
  onRecord(listener: Parameters<Store["onRecord"]>[0]): () => void {
    const stop = this.ctx.effect(() => suite.onRecord(listener), "daemon.onRecord");
    return () => void stop();
  }
}

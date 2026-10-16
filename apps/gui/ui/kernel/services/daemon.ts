// The `daemon` service: the window's connection to the daemon and the session it shows, from the
// `suite` store. Everything it reads is reactive; the store stays where components that are not
// plugins yet read it. No `$state` of its own: the store has it (see the README).
import { Service, type Context } from "@neoworks/extension-system";
import { suite } from "../../state/suite.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    daemon: DaemonService;
  }
}

export class DaemonService extends Service {
  constructor(ctx: Context) {
    super(ctx, "daemon");
  }

  /** Connects to the daemon and opens the session `suite gui` asked for. */
  start() {
    return suite.start();
  }

  get connection() {
    return suite.connection;
  }
  /** Why starting or the last request failed. */
  get error() {
    return suite.error;
  }
  set error(message: string | undefined) {
    suite.error = message;
  }
  /** What the window draws with. */
  get palette() {
    return suite.palette;
  }
  /** Every session, kept current by the daemon. */
  get sessions() {
    return suite.sessions;
  }
  /** The session this window shows, and its directory. */
  get sessionId() {
    return suite.sessionId;
  }
  get session() {
    return suite.session;
  }
  get cwd() {
    return suite.cwd;
  }
  /** The shown session's chat, as the daemon resolved it. */
  get items() {
    return suite.items;
  }
  get status() {
    return suite.status;
  }
  get branch() {
    return suite.branch;
  }

  open(id: string) {
    return suite.open(id);
  }
  newSession(cwd?: string) {
    return suite.newSession(cwd);
  }
  send(text: string) {
    return suite.send(text);
  }
  cancel() {
    return suite.cancel();
  }
  cycleApprovals() {
    return suite.cycleApprovals();
  }
  assetSrc(src: string) {
    return suite.assetSrc(src);
  }

  /** Every event of the shown session from now on; stops when the plugin unloads. */
  onRecord(listener: Parameters<typeof suite.onRecord>[0]): () => void {
    return this.ctx.effect(() => suite.onRecord(listener), "daemon.onRecord");
  }
}

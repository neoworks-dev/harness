// The client an app holds: requests to the daemon, and the values and subscriptions it keeps
// current. The wire work is in connection.ts, the following of sessions in subscriptions.ts.
import type {
  AgentFocus,
  AgentMove,
  AppPresence,
  Asset,
  AssetPut,
  ConnectorInfo,
  Envelope,
  FileChanged,
  SessionCreateParams,
  SessionInfo,
  SessionInput,
  SessionSettingsPayload,
  Theme,
  UiAction,
} from "@suite/protocol/types";
import type { AddressSource } from "./address.ts";
import { Broadcasts } from "./broadcasts.ts";
import { Connection, type ConnectionState } from "./connection.ts";
import { reportUnlessDisconnected } from "./errors.ts";
import {
  Subscriptions,
  type SubscribeOptions,
  type ViewSubscribeOptions,
} from "./subscriptions.ts";
import type { Unsubscribe, Watched } from "./watched.ts";

const DEFAULT_REQUEST_TIMEOUT_MS = 10_000;

export interface ConnectOptions {
  /** Your client's name, sent to the daemon in hello, for example `notes`. */
  client: string;
  /**
   * Where the daemon is. Called on every (re)connect, because a restarted daemon has a new port
   * and token. Use `daemonJsonAddress` (from `@suite/sdk/node`) on the daemon's machine, or
   * `fixedAddress` when you were told the address.
   */
  address: AddressSource;
  /** Registered on connect and again after every reconnect. Change it with `setPresence`. */
  presence?: AppPresence;
  /** Reconnect after the connection drops (default true). */
  reconnect?: boolean;
  /**
   * If the daemon isn't running at first, keep trying until it is, instead of rejecting. Lets an
   * app start before the daemon (default false).
   */
  waitForDaemon?: boolean;
  /**
   * How long to wait for the daemon's answer to a request before rejecting with `TimeoutError`
   * (default 10000).
   */
  requestTimeoutMs?: number;
  /**
   * Called with failures nobody is waiting for: a presence the daemon refuses on reconnect, a
   * subscription it refuses, an error thrown by one of your callbacks.
   */
  onError?: (error: Error) => void;
}

export interface InputOptions {
  /** The app the input comes from; defaults to the presence's app, else the client name. */
  app?: string;
  /** Absolute paths of images to attach. */
  images?: string[];
}

/**
 * Connects to the daemon, says hello and registers the presence, if any.
 *
 * Rejects with `DisconnectedError` if the daemon cannot be reached (unless `waitForDaemon` is
 * set), and with `SuiteError` if it refuses the client or the presence.
 */
export function connect(options: ConnectOptions): Promise<Client> {
  return Client.open(options);
}

/**
 * A connection to the suite daemon. Get one from `connect`. It survives a daemon restart:
 * it reconnects, registers the presence again, and resumes every subscription.
 *
 * Requests reject with `SuiteError` when the daemon refuses them, with `DisconnectedError` when
 * there is no connection (also for requests still waiting when it drops), and with
 * `TimeoutError` when the daemon does not answer.
 */
export class Client {
  /**
   * Where the connection is: `connecting`, `open`, `reconnecting` after a drop, or `closed` for
   * good. Becomes `open` once a (re)connect is fully set up.
   */
  readonly state: Watched<ConnectionState>;
  /**
   * Where the agent lives: the current session and the app showing the agent (no app: nowhere).
   * Undefined until the daemon's first `agent.focus`, which it sends after every (re)connect and
   * on every change.
   */
  readonly focus: Watched<AgentFocus | undefined>;
  /** The daemon's theme. Undefined until the daemon sends it, which it does after every (re)connect. */
  readonly theme: Watched<Theme | undefined>;
  /** Every connector the daemon knows. Empty until the list arrives after a (re)connect. */
  readonly connectors: Watched<ConnectorInfo[]>;
  /**
   * Every session, kept current as the daemon announces changes (state, title, preview). Empty
   * until the list arrives after a (re)connect. Sessions you create yourself are added at once.
   */
  readonly sessions: Watched<SessionInfo[]>;

  private readonly connection: Connection;
  private readonly broadcasts: Broadcasts;
  private readonly subscriptions: Subscriptions;
  private presence?: AppPresence;

  private constructor(
    private readonly options: ConnectOptions,
    private readonly reportError: (error: Error) => void,
  ) {
    this.presence = options.presence;
    this.broadcasts = new Broadcasts(reportError);
    this.connection = new Connection({
      name: options.client,
      address: options.address,
      reconnect: options.reconnect !== false,
      waitForDaemon: options.waitForDaemon === true,
      replyTimeoutMs: options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS,
      introduce: () => this.registerPresence(),
      onFrame: (frame) => this.take(frame),
      onError: reportError,
    });
    this.subscriptions = new Subscriptions({ connection: this.connection, onError: reportError });
    this.state = this.connection.state;
    this.focus = this.broadcasts.focus;
    this.theme = this.broadcasts.theme;
    this.connectors = this.broadcasts.connectors;
    this.sessions = this.broadcasts.sessions;
    this.state.on((state) => {
      if (state === "open") this.resumeAfterConnect();
    });
  }

  /** Use `connect`. */
  static async open(options: ConnectOptions): Promise<Client> {
    const reportError = (error: Error): void => options.onError?.(error);
    const client = new Client(options, reportError);
    await client.connection.start();
    return client;
  }

  /**
   * Moves the agent to `move.app` (an open connector of kind app), or home when `app` is omitted.
   * With `move.session`, that session also becomes the current one for everybody. Resolves once
   * the daemon has acted; rejects with `SuiteError` `not_found` for an unknown app or session.
   */
  async moveAgent(move: AgentMove = {}): Promise<void> {
    await this.connection.request("agent.move", move);
  }

  /**
   * Sets the presence: sent now (if connected) and again after every reconnect. Send
   * `open: false` to say the app is closing.
   */
  async setPresence(presence: AppPresence): Promise<void> {
    this.presence = presence;
    if (this.isOpen()) await this.registerPresence();
  }

  /**
   * Tells the daemon what the app shows now (the presence `context`, for example the open file)
   * without sending the whole presence again. The agent sees it at its next turn. Cheap enough
   * to call on every cursor move; kept for reconnects. Needs a presence, so it rejects with a
   * plain `Error` if there is none yet.
   */
  async setContext(context: Record<string, string>): Promise<void> {
    if (!this.presence) throw new Error("setContext needs a presence; call setPresence first");
    this.presence = { ...this.presence, context };
    if (!this.isOpen()) return;
    await this.connection.request("app.context", { app: this.presence.app, context });
  }

  /** Fetches the sessions from the daemon. Also what `sessions` is filled from. */
  async listSessions(): Promise<SessionInfo[]> {
    const { sessions } = await this.connection.request("session.list");
    return sessions;
  }

  /**
   * The harnesses `session` can switch to, each with what it offers to set (model, effort, mode)
   * and the current values, for a picker. A harness that could not be asked has `options: []`
   * and an `error`. Setting is done by sending `/model [harness] <value>`, `/effort`, `/mode` or
   * `/harness <name>` as the session's input. Asking may start a guest's adapter, so it can take
   * seconds.
   */
  sessionSettings(session: string): Promise<SessionSettingsPayload> {
    return this.connection.request("session.settings", { session });
  }

  /** Starts a new session (in `params.cwd`, by default the daemon's home directory). */
  createSession(params: SessionCreateParams = {}): Promise<SessionInfo> {
    return this.connection.request("session.create", params);
  }

  /** Fetches the connectors from the daemon. Also what `connectors` is filled from. */
  async listConnectors(): Promise<ConnectorInfo[]> {
    const { connectors } = await this.connection.request("connectors.list");
    return connectors;
  }

  /**
   * Replays the session's events from `options.fromSeq`, then follows it live. After a
   * reconnect it carries on from the last event delivered, so nothing is missed or repeated.
   * Works while disconnected too: it starts when the connection opens.
   *
   * `onEvent` and `onLive` fire as described in `SubscribeOptions`. If the daemon refuses (an
   * unknown session), the error goes to `options.onError`, else to the client's `onError`.
   * Returns the function that stops it.
   */
  subscribe(session: string, options: SubscribeOptions): Unsubscribe {
    return this.subscriptions.followEvents(session, options);
  }

  /**
   * Follows the resolved view of a session (or, with `options.agent`, of one agent's work in
   * it): `onSnapshot` first, then `onPatch` for each change. After a reconnect the client
   * subscribes again and a fresh snapshot arrives, so replace your state with it. Patches are
   * never delivered before the snapshot they apply to. Works while disconnected too: it starts
   * when the connection opens.
   *
   * If the daemon refuses (an unknown session), the error goes to `options.onError`, else to the
   * client's `onError`. Returns the function that stops it.
   */
  subscribeView(session: string, options: ViewSubscribeOptions): Unsubscribe {
    return this.subscriptions.followView(session, options);
  }

  /**
   * Reports that a button was pressed in the view item `ref`. `copy:` actions are the
   * renderer's to handle and never reach the daemon.
   */
  async action(session: string, ref: string, action: string, args?: unknown): Promise<void> {
    const payload: UiAction = { session, ref, action };
    if (args !== undefined) payload.args = args;
    await this.connection.request("ui.action", payload);
  }

  /**
   * Sends text (and images) to the session's agent as user input. Resolves once the daemon has
   * queued it, not when the agent has answered; follow the answer with `subscribe`.
   */
  async input(session: string, text: string, options: InputOptions = {}): Promise<void> {
    const payload: SessionInput = { session, app: options.app ?? this.defaultApp(), text };
    if (options.images && options.images.length > 0) payload.images = options.images;
    await this.connection.request("session.input", payload);
  }

  /** Cancels the turn that is running in the session. Inputs queued behind it still run. */
  async cancel(session: string): Promise<void> {
    await this.connection.request("session.cancel", { session });
  }

  /**
   * Stores a file (`asset.path`, copied in) or bytes (`asset.data`, base64) in the daemon's
   * asset store and returns the asset. Its `handle` (`asset://…`) can be given to other apps and
   * tools. Identical content gets the same handle.
   */
  putAsset(asset: AssetPut): Promise<Asset> {
    return this.connection.request("asset.put", asset);
  }

  /** Looks up an asset by handle. Rejects with `SuiteError` `not_found` for an unknown one. */
  getAsset(handle: string): Promise<Asset> {
    return this.connection.request("asset.get", { handle });
  }

  /**
   * Calls `callback` when another client or the agent changed a file: reload it, or show a
   * diff. Your own `fileChanged` is not sent back to you. Returns the function that stops it.
   */
  onFileChanged(callback: (change: FileChanged) => void): Unsubscribe {
    return this.broadcasts.fileChanges.add(callback);
  }

  /**
   * Tells every other client that you changed the file at `path` (absolute), for example after
   * saving it. Throws `DisconnectedError` when there is no connection.
   */
  fileChanged(path: string): void {
    this.connection.send({ topic: "file.changed", payload: { path, app: this.defaultApp() } });
  }

  /**
   * Marks the app as closed (presence `open: false`), then closes the connection for good.
   * Subscriptions end, and calls made afterwards reject with `DisconnectedError`.
   */
  async close(): Promise<void> {
    if (this.state.value === "closed") return;
    if (this.isOpen() && this.presence) {
      const closing = { ...this.presence, open: false };
      await this.connection.request("app.presence", closing).catch(() => {});
    }
    this.subscriptions.clear();
    this.connection.close();
  }

  private isOpen(): boolean {
    return this.state.value === "open";
  }

  private defaultApp(): string {
    if (this.presence) return this.presence.app;
    return this.options.client;
  }

  private async registerPresence(): Promise<void> {
    if (this.presence) await this.connection.request("app.presence", this.presence);
  }

  private take(frame: Envelope): void {
    this.broadcasts.take(frame);
    this.subscriptions.take(frame);
  }

  /**
   * The daemon forgets everything when it restarts, and a dropped connection loses what was sent
   * on it: follow the subscriptions again and refetch the lists. The replies fill `connectors`
   * and `sessions`.
   */
  private resumeAfterConnect(): void {
    this.subscriptions.resume();
    this.connection
      .request("connectors.list")
      .catch((thrown: unknown) => reportUnlessDisconnected(thrown, this.reportError));
    this.connection
      .request("session.list")
      .catch((thrown: unknown) => reportUnlessDisconnected(thrown, this.reportError));
  }
}

// The bus protocol client, shared by the Bun entry (client.ts) and the browser entry (browser.ts).
// Imports nothing from node: it only needs a global WebSocket and a way to find the daemon's URL
// (`resolveUrl`, supplied by the subclass).
//
// The client survives daemon restarts: it reconnects with backoff, re-sends hello and the current
// presence, resumes every live subscription from the last seq it delivered and re-subscribes every
// view (the daemon answers with a fresh snapshot).
import type {
  AgentFocus,
  AgentMove,
  AppPresence,
  Asset,
  AssetPut,
  ConnectorInfo,
  Envelope,
  ErrorCode,
  FileChanged,
  SessionCreateParams,
  SessionEvent,
  SessionEventRecord,
  SessionInfo,
  SessionInput,
  Theme,
  UiAction,
  ViewPatch,
  ViewSnapshot,
} from "@suite/protocol/types";

const PROTOCOL_VERSION = 1;
const INITIAL_BACKOFF_MS = 250;
const MAX_BACKOFF_MS = 5000;
const REQUEST_TIMEOUT_MS = 10_000;

/** The daemon answered a request with bus.error. */
export class SuiteError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "SuiteError";
  }
}

/** There is no live connection to the daemon (never connected, or it dropped). */
export class DisconnectedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DisconnectedError";
  }
}

export interface BusOptions {
  /** Client name sent in hello, e.g. overlay. */
  client: string;
  /** Registered on connect and again after every reconnect. */
  presence?: AppPresence;
  /** Reconnect after the connection drops (default true). */
  reconnect?: boolean;
  /**
   * If the daemon isn't running at first, keep retrying until it is instead of rejecting, so an
   * app can start before the daemon (default false).
   */
  waitForDaemon?: boolean;
  /** Failures nobody is waiting for: a rejected presence on reconnect, a failed resume. */
  onError?: (error: Error) => void;
}

export interface SubscribeOptions {
  /** First seq to deliver (inclusive); defaults to 1, i.e. the whole history. */
  fromSeq?: number;
  /** Each event once, in seq order, across reconnects. */
  onEvent: (event: SessionEvent, record: SessionEventRecord) => void;
  /** The replay is over and events are live. Fires again after each reconnect. */
  onLive?: (session: SessionInfo) => void;
  /** The daemon refused the subscription (e.g. unknown session). */
  onError?: (error: Error) => void;
}

export interface ViewSubscribeOptions {
  /**
   * The whole view as of `snapshot.seq`: replaces whatever the caller had. Fires again after each
   * reconnect (the daemon answers a re-subscribe with a fresh snapshot).
   */
  onSnapshot: (snapshot: ViewSnapshot) => void;
  /** Changes after the latest snapshot, in order. Never delivered before a snapshot. */
  onPatch: (patch: ViewPatch) => void;
  /** The daemon refused the subscription (e.g. unknown session). */
  onError?: (error: Error) => void;
}

export interface InputOptions {
  /** App the input comes from; defaults to the presence's app, else the client name. */
  app?: string;
  /** Absolute paths of images to attach. */
  images?: string[];
}

interface Pending {
  resolve: (reply: Envelope) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface Subscription {
  session: string;
  /** The next seq to deliver; also where a resume starts. */
  next: number;
  options: SubscribeOptions;
}

interface ViewSubscription {
  session: string;
  /** True from sending view.subscribe until its snapshot arrives; patches in between are stale. */
  awaitingSnapshot: boolean;
  options: ViewSubscribeOptions;
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

function asError(err: unknown): Error {
  if (err instanceof Error) return err;
  return new Error(String(err));
}

export class BusClient {
  private ws?: WebSocket;
  private connected = false;
  private closing = false;
  private reconnecting = false;
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();
  private readonly subscriptions = new Set<Subscription>();
  private readonly listeners = new Set<(connected: boolean) => void>();
  private presence?: AppPresence;
  private latestFocus?: AgentFocus;
  private latestConnectors: ConnectorInfo[] = [];
  private readonly focusListeners = new Set<(focus: AgentFocus) => void>();
  private readonly connectorListeners = new Set<(connectors: ConnectorInfo[]) => void>();
  private latestTheme?: Theme;
  private readonly themeListeners = new Set<(theme: Theme) => void>();
  private readonly viewSubscriptions = new Set<ViewSubscription>();
  private readonly fileListeners = new Set<(change: FileChanged) => void>();
  private welcome?: { resolve: () => void; reject: (error: Error) => void };
  /** Aborted by close() to cut short any backoff sleep. */
  private readonly stop = new AbortController();

  /**
   * `resolveUrl` returns the `ws://` URL (token included) to dial; it runs on every (re)connect,
   * since the daemon's port and token change when it restarts.
   */
  protected constructor(
    private readonly options: BusOptions,
    private readonly resolveUrl: () => Promise<string>,
  ) {
    this.presence = options.presence;
  }

  /**
   * Connects, says hello and registers the presence. Rejects if the daemon isn't running, unless
   * `waitForDaemon` is set. Subclasses expose it as `connect`.
   */
  protected static async start<T extends BusClient>(client: T): Promise<T> {
    const { options } = client;
    let backoff = INITIAL_BACKOFF_MS;
    for (;;) {
      try {
        await client.open();
        return client;
      } catch (err) {
        // A daemon that answers but refuses us won't change its mind; only a missing one might.
        const retry = options.waitForDaemon === true && !(err instanceof SuiteError);
        if (!retry || client.closing) throw err;
        await sleep(backoff, client.stop.signal);
        backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
      }
    }
  }

  get isConnected(): boolean {
    return this.connected;
  }

  /** Called with true after each (re)connect is fully set up, false when the connection drops. */
  onConnectionChange(cb: (connected: boolean) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  /**
   * Where the agent lives: the latest `agent.focus` from the daemon, which sends one after each
   * (re)connect and another on every change. Undefined until the first one arrives; see
   * `waitForFocus`.
   */
  get focus(): AgentFocus | undefined {
    return this.latestFocus;
  }

  /** Called with every `agent.focus` frame, including the one the daemon sends after a reconnect. */
  onFocus(cb: (focus: AgentFocus) => void): () => void {
    this.focusListeners.add(cb);
    return () => this.focusListeners.delete(cb);
  }

  /** Resolves with the current focus, waiting for the first one if it hasn't arrived yet. */
  async waitForFocus(timeoutMs = REQUEST_TIMEOUT_MS): Promise<AgentFocus> {
    const known = this.latestFocus;
    if (known) return known;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        off();
        reject(new Error("no agent.focus from the daemon"));
      }, timeoutMs);
      const off = this.onFocus((focus) => {
        clearTimeout(timer);
        off();
        resolve(focus);
      });
    });
  }

  /**
   * The latest `theme` from the daemon, which sends one after each (re)connect and whenever it
   * changes. Undefined until the first one arrives; see `waitForTheme`.
   */
  get theme(): Theme | undefined {
    return this.latestTheme;
  }

  /** Called with every `theme` frame, including the one the daemon sends after a reconnect. */
  onTheme(cb: (theme: Theme) => void): () => void {
    this.themeListeners.add(cb);
    return () => this.themeListeners.delete(cb);
  }

  /** Resolves with the current theme, waiting for the first one if it hasn't arrived yet. */
  async waitForTheme(timeoutMs = REQUEST_TIMEOUT_MS): Promise<Theme> {
    const known = this.latestTheme;
    if (known) return known;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        off();
        reject(new Error("no theme from the daemon"));
      }, timeoutMs);
      const off = this.onTheme((theme) => {
        clearTimeout(timer);
        off();
        resolve(theme);
      });
    });
  }

  /**
   * Every connector the daemon knows, as of the latest `connectors.changed` (or connectors.list
   * reply). Fetched once after each (re)connect, so it is empty only until that reply arrives.
   */
  get connectors(): ConnectorInfo[] {
    return this.latestConnectors;
  }

  /** Called with the full connector list whenever it changes (and after the first fetch). */
  onConnectors(cb: (connectors: ConnectorInfo[]) => void): () => void {
    this.connectorListeners.add(cb);
    return () => this.connectorListeners.delete(cb);
  }

  /**
   * Moves the agent: to `app` (an open connector of kind app), or home when `app` is omitted.
   * With `session`, that session becomes the current one for everybody. Resolves once the daemon
   * has broadcast the resulting `agent.focus`.
   */
  async moveAgent(move: AgentMove = {}): Promise<void> {
    await this.call((id) => ({ topic: "agent.move", id, payload: move }));
  }

  /** Sets the presence now (if connected) and on every reconnect. Use `open: false` on close. */
  async setPresence(presence: AppPresence): Promise<void> {
    this.presence = presence;
    if (this.connected) await this.sendPresence(presence);
  }

  async listSessions(): Promise<SessionInfo[]> {
    const reply = await this.call((id) => ({ topic: "session.list", id }));
    if (reply.topic !== "session.list_result") throw unexpected(reply);
    return reply.payload.sessions;
  }

  async createSession(params: SessionCreateParams = {}): Promise<SessionInfo> {
    const reply = await this.call((id) => ({ topic: "session.create", id, payload: params }));
    if (reply.topic !== "session.info") throw unexpected(reply);
    return reply.payload;
  }

  /**
   * Replays the session from `fromSeq`, then follows it live. Survives reconnects without gaps or
   * duplicates. Works while disconnected too: it starts when the connection does.
   */
  subscribe(session: string, options: SubscribeOptions): { close(): void } {
    const sub: Subscription = { session, next: options.fromSeq ?? 1, options };
    this.subscriptions.add(sub);
    if (this.connected) this.sendSubscribe(sub);
    return {
      close: () => {
        if (!this.subscriptions.delete(sub)) return;
        const shared = [...this.subscriptions].some((other) => other.session === session);
        // The daemon tracks one subscription per session and connection.
        if (this.connected && !shared) {
          this.call((id) => ({ topic: "session.unsubscribe", id, payload: { session } })).catch(
            () => {},
          );
        }
      },
    };
  }

  /**
   * Follows the resolved view of a session: a snapshot first, then a patch per change. Survives
   * reconnects by re-subscribing, which brings a fresh snapshot (the caller replaces its state).
   * Works while disconnected too: it starts when the connection does.
   */
  subscribeView(session: string, options: ViewSubscribeOptions): { close(): void } {
    const sub: ViewSubscription = { session, awaitingSnapshot: true, options };
    this.viewSubscriptions.add(sub);
    if (this.connected) this.sendViewSubscribe(sub);
    return {
      close: () => {
        if (!this.viewSubscriptions.delete(sub)) return;
        const shared = [...this.viewSubscriptions].some((other) => other.session === session);
        // The daemon tracks one view subscription per session and connection.
        if (this.connected && !shared) {
          this.call((id) => ({ topic: "view.unsubscribe", id, payload: { session } })).catch(
            () => {},
          );
        }
      },
    };
  }

  /** A button was pressed in view item `ref`. `copy:` actions are the renderer's to handle. */
  async action(session: string, ref: string, action: string, args?: unknown): Promise<void> {
    const payload: UiAction = { session, ref, action };
    if (args !== undefined) payload.args = args;
    await this.call((id) => ({ topic: "ui.action", id, payload }));
  }

  async input(session: string, text: string, options: InputOptions = {}): Promise<void> {
    const payload: SessionInput = { session, app: options.app ?? this.defaultApp(), text };
    if (options.images && options.images.length > 0) payload.images = options.images;
    await this.call((id) => ({ topic: "session.input", id, payload }));
  }

  /** Cancels the running turn; queued inputs still run. */
  async cancel(session: string): Promise<void> {
    await this.call((id) => ({ topic: "session.cancel", id, payload: { session } }));
  }

  /**
   * Stores a file (`path`, copied in) or bytes (`data`, base64) in the daemon's asset store and
   * returns the asset, whose `handle` (`asset://…`) other apps and tools can use. Identical
   * content gets the same handle.
   */
  async putAsset(asset: AssetPut): Promise<Asset> {
    const reply = await this.call((id) => ({ topic: "asset.put", id, payload: asset }));
    if (reply.topic !== "asset.info") throw unexpected(reply);
    return reply.payload;
  }

  /** Looks up a handle; rejects with `SuiteError` `not_found` for an unknown one. */
  async getAsset(handle: string): Promise<Asset> {
    const reply = await this.call((id) => ({ topic: "asset.get", id, payload: { handle } }));
    if (reply.topic !== "asset.info") throw unexpected(reply);
    return reply.payload;
  }

  /**
   * Called when another client or the agent changed a file (`file.changed`): reload it or show a
   * diff. Your own `fileChanged` is not echoed back.
   */
  onFileChanged(cb: (change: FileChanged) => void): () => void {
    this.fileListeners.add(cb);
    return () => this.fileListeners.delete(cb);
  }

  /** Tells every other client that you changed the file at `path` (absolute), e.g. after saving. */
  fileChanged(path: string): void {
    const payload: FileChanged = { path, app: this.defaultApp() };
    this.send({ topic: "file.changed", payload });
  }

  async listConnectors(): Promise<ConnectorInfo[]> {
    const reply = await this.call((id) => ({ topic: "connectors.list", id }));
    if (reply.topic !== "connectors.list_result") throw unexpected(reply);
    return reply.payload.connectors;
  }

  /** Unregisters the presence (open: false), then closes the connection for good. */
  async close(): Promise<void> {
    if (this.closing) return;
    if (this.connected && this.presence) {
      await this.sendPresence({ ...this.presence, open: false }).catch(() => {});
    }
    this.closing = true;
    this.stop.abort();
    this.subscriptions.clear();
    this.viewSubscriptions.clear();
    this.ws?.close();
    this.drop(new DisconnectedError("client closed"));
  }

  private defaultApp(): string {
    if (this.presence) return this.presence.app;
    return this.options.client;
  }

  private emit(connected: boolean): void {
    for (const cb of this.listeners) cb(connected);
  }

  /** Dials the daemon named in daemon.json and sets the session up. Throws if any step fails. */
  private async open(): Promise<void> {
    const ws = await dial(await this.resolveUrl());
    this.ws = ws;
    ws.onmessage = (event) => this.receive(String(event.data));
    ws.onclose = () => {
      if (this.ws !== ws) return;
      const wasConnected = this.connected;
      this.drop(new DisconnectedError("connection to the suite daemon closed"));
      // A failed first open() is handled by its caller; only a lost session needs resuming.
      if (wasConnected && this.options.reconnect !== false && !this.closing) void this.reconnect();
    };
    try {
      await this.hello();
      if (this.presence) await this.sendPresence(this.presence);
    } catch (err) {
      this.ws = undefined;
      ws.close();
      throw err;
    }
    this.connected = true;
    for (const sub of this.subscriptions) this.sendSubscribe(sub);
    for (const sub of this.viewSubscriptions) this.sendViewSubscribe(sub);
    this.seedConnectors();
    this.emit(true);
  }

  private hello(): Promise<void> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new DisconnectedError("no welcome from the daemon")),
        REQUEST_TIMEOUT_MS,
      );
      this.welcome = {
        resolve: () => {
          clearTimeout(timer);
          resolve();
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      };
      this.send({
        topic: "bus.hello",
        payload: { protocol: PROTOCOL_VERSION, client: this.options.client },
      });
    });
  }

  private async reconnect(): Promise<void> {
    if (this.reconnecting) return;
    this.reconnecting = true;
    let backoff = INITIAL_BACKOFF_MS;
    try {
      while (!this.closing) {
        await sleep(backoff, this.stop.signal);
        if (this.closing) return;
        try {
          await this.open();
          return;
        } catch (err) {
          // The daemon answered but refused us (e.g. an invalid presence); retrying won't help.
          if (err instanceof SuiteError) {
            this.options.onError?.(err);
            return;
          }
          backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
        }
      }
    } finally {
      this.reconnecting = false;
    }
  }

  /** Connection lost: fail what was waiting on it. Subscriptions stay, to be resumed. */
  private drop(reason: Error): void {
    const wasConnected = this.connected;
    this.connected = false;
    this.ws = undefined;
    this.welcome?.reject(reason);
    this.welcome = undefined;
    for (const [id, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(reason);
      this.pending.delete(id);
    }
    if (wasConnected) this.emit(false);
  }

  private async sendPresence(presence: AppPresence): Promise<void> {
    await this.call((id) => ({ topic: "app.presence", id, payload: presence }));
  }

  /** Asks for the connector list; the reply is picked up in receive like any other snapshot. */
  private seedConnectors(): void {
    this.listConnectors().catch(() => {});
  }

  private setConnectors(connectors: ConnectorInfo[]): void {
    this.latestConnectors = connectors;
    for (const cb of [...this.connectorListeners]) {
      try {
        cb(connectors);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }

  private setFocus(focus: AgentFocus): void {
    this.latestFocus = focus;
    for (const cb of [...this.focusListeners]) {
      try {
        cb(focus);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }

  private setTheme(theme: Theme): void {
    this.latestTheme = theme;
    for (const cb of [...this.themeListeners]) {
      try {
        cb(theme);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }

  private sendViewSubscribe(sub: ViewSubscription): void {
    sub.awaitingSnapshot = true;
    const { session } = sub;
    // The snapshot is the reply; receive() hands it to the subscription, so only failures matter.
    this.call((id) => ({ topic: "view.subscribe", id, payload: { session } })).catch(
      (err: unknown) => {
        // A dropped connection resubscribes on its own.
        if (err instanceof DisconnectedError) return;
        const error = asError(err);
        if (sub.options.onError) sub.options.onError(error);
        else this.options.onError?.(error);
      },
    );
  }

  private sendSubscribe(sub: Subscription): void {
    const { session, next } = sub;
    this.call((id) => ({ topic: "session.subscribe", id, payload: { session, fromSeq: next } }))
      .then((reply) => {
        if (reply.topic === "session.subscribed") sub.options.onLive?.(reply.payload.session);
      })
      .catch((err: unknown) => {
        // A dropped connection resumes the subscription on its own.
        if (err instanceof DisconnectedError) return;
        const error = asError(err);
        if (sub.options.onError) sub.options.onError(error);
        else this.options.onError?.(error);
      });
  }

  private send(envelope: Envelope): void {
    if (!this.ws) throw new DisconnectedError("not connected to the suite daemon");
    this.ws.send(JSON.stringify(envelope));
  }

  /** Sends a request and waits for the frame carrying its id. */
  private call(build: (id: string) => Envelope): Promise<Envelope> {
    if (!this.ws) {
      return Promise.reject(new DisconnectedError("not connected to the suite daemon"));
    }
    const id = String(this.nextId++);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`request ${id} timed out`));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.send(build(id));
      } catch (err) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(asError(err));
      }
    });
  }

  private receive(frame: string): void {
    let msg: Envelope;
    try {
      msg = JSON.parse(frame);
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null) return;
    switch (msg.topic) {
      case "bus.welcome":
        this.welcome?.resolve();
        this.welcome = undefined;
        return;
      case "session.events":
        this.deliver(msg.payload);
        return;
      case "agent.focus":
        this.setFocus(msg.payload);
        return;
      case "theme":
        this.setTheme(msg.payload);
        return;
      case "file.changed":
        for (const cb of [...this.fileListeners]) {
          try {
            cb(msg.payload);
          } catch (err) {
            this.options.onError?.(asError(err));
          }
        }
        return;
      case "view.snapshot":
        this.deliverSnapshot(msg.payload);
        this.settle(msg.id, msg);
        return;
      case "view.patch":
        this.deliverPatch(msg.payload);
        return;
      case "connectors.changed":
        this.setConnectors(msg.payload.connectors);
        return;
      case "connectors.list_result":
        // A fresh snapshot, in wire order with the broadcasts, so the latest one is the truth.
        this.setConnectors(msg.payload.connectors);
        this.settle(msg.id, msg);
        return;
      case "bus.error":
        if (msg.id === undefined) {
          // Not tied to a request: a failed hello, or something the daemon couldn't parse.
          const error = new SuiteError(msg.payload.code, msg.payload.message);
          if (this.welcome) {
            this.welcome.reject(error);
            this.welcome = undefined;
          } else this.options.onError?.(error);
          return;
        }
        this.settle(msg.id, msg);
        return;
      default:
        // Replies carry the id of their request; anything else (unknown topics) is ignored.
        if ("id" in msg && typeof msg.id === "string") this.settle(msg.id, msg);
    }
  }

  private settle(id: string, reply: Envelope): void {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    clearTimeout(pending.timer);
    if (reply.topic === "bus.error") {
      pending.reject(new SuiteError(reply.payload.code, reply.payload.message));
    } else pending.resolve(reply);
  }

  private deliverSnapshot(snapshot: ViewSnapshot): void {
    for (const sub of [...this.viewSubscriptions]) {
      if (sub.session !== snapshot.session) continue;
      sub.awaitingSnapshot = false;
      try {
        sub.options.onSnapshot(snapshot);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }

  private deliverPatch(patch: ViewPatch): void {
    for (const sub of [...this.viewSubscriptions]) {
      if (sub.session !== patch.session || sub.awaitingSnapshot) continue;
      try {
        sub.options.onPatch(patch);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }

  /** Hands a record to every subscriber of its session that hasn't seen it yet. */
  private deliver(record: SessionEventRecord): void {
    for (const sub of [...this.subscriptions]) {
      // Replays requested by another subscription of the same session arrive here too.
      if (sub.session !== record.session || record.seq < sub.next) continue;
      sub.next = record.seq + 1;
      try {
        sub.options.onEvent(record.event, record);
      } catch (err) {
        this.options.onError?.(asError(err));
      }
    }
  }
}

function unexpected(reply: Envelope): Error {
  return new Error(`unexpected ${reply.topic} reply`);
}

/** Opens a WebSocket; rejects if it can't be established. */
function dial(url: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    ws.onopen = () => resolve(ws);
    ws.onerror = () => reject(new DisconnectedError("could not reach the suite daemon"));
    ws.onclose = () => reject(new DisconnectedError("could not reach the suite daemon"));
  });
}

// Bus client for TS apps. Bun's WebSocket can't dial Unix sockets, so it uses the daemon's
// loopback listener: daemon.json (next to the socket) holds the URL and a token that change on
// every daemon start, so it is re-read on each (re)connect.
//
// The client survives daemon restarts: it reconnects with backoff, re-sends hello and the current
// presence, and resumes every live subscription from the last seq it delivered.
import { readFile } from "node:fs/promises";
import { userInfo } from "node:os";
import { dirname, join } from "node:path";
import type {
  AppPresence,
  ConnectorInfo,
  Envelope,
  ErrorCode,
  SessionCreateParams,
  SessionEvent,
  SessionEventRecord,
  SessionInfo,
  SessionInput,
} from "@suite/protocol/types";

const PROTOCOL_VERSION = 1;
const INITIAL_BACKOFF_MS = 250;
const MAX_BACKOFF_MS = 5000;
const REQUEST_TIMEOUT_MS = 10_000;

type Env = Record<string, string | undefined>;

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

export interface DaemonEndpoint {
  /** `ws://127.0.0.1:PORT/bus?token=…` */
  wsUrl: string;
  /** `http://127.0.0.1:PORT`, also serves the gateway at /mcp. */
  url: string;
  token: string;
}

/** Where the daemon puts daemon.json: next to its socket (see daemon/src/paths.ts). */
export function daemonInfoPath(env: Env = process.env): string {
  let socket = env.SUITE_SOCKET;
  if (!socket && env.XDG_RUNTIME_DIR) socket = join(env.XDG_RUNTIME_DIR, "suite", "bus.sock");
  if (!socket) socket = join("/tmp", `suite-${userInfo().uid}`, "bus.sock");
  return join(dirname(socket), "daemon.json");
}

export async function readDaemonEndpoint(env: Env = process.env): Promise<DaemonEndpoint> {
  const path = daemonInfoPath(env);
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch {
    throw new DisconnectedError(`the suite daemon is not running (no ${path})`);
  }
  const info: { url?: unknown; token?: unknown } = JSON.parse(raw);
  if (typeof info.url !== "string" || typeof info.token !== "string") {
    throw new DisconnectedError(`${path} is not a daemon info file`);
  }
  const wsBase = info.url.replace(/^http/, "ws");
  return {
    url: info.url,
    token: info.token,
    wsUrl: `${wsBase}/bus?token=${encodeURIComponent(info.token)}`,
  };
}

export interface ConnectOptions {
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
  /** Where to look for the daemon (SUITE_SOCKET etc.); defaults to process.env. */
  env?: Env;
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

export class SuiteClient {
  private ws?: WebSocket;
  private connected = false;
  private closing = false;
  private reconnecting = false;
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();
  private readonly subscriptions = new Set<Subscription>();
  private readonly listeners = new Set<(connected: boolean) => void>();
  private presence?: AppPresence;
  private welcome?: { resolve: () => void; reject: (error: Error) => void };
  /** Aborted by close() to cut short any backoff sleep. */
  private readonly stop = new AbortController();

  private constructor(private readonly options: ConnectOptions) {
    this.presence = options.presence;
  }

  /**
   * Connects, says hello and registers the presence. Rejects if the daemon isn't running, unless
   * `waitForDaemon` is set.
   */
  static async connect(options: ConnectOptions): Promise<SuiteClient> {
    const client = new SuiteClient(options);
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

  async input(session: string, text: string, options: InputOptions = {}): Promise<void> {
    const payload: SessionInput = { session, app: options.app ?? this.defaultApp(), text };
    if (options.images && options.images.length > 0) payload.images = options.images;
    await this.call((id) => ({ topic: "session.input", id, payload }));
  }

  /** Cancels the running turn; queued inputs still run. */
  async cancel(session: string): Promise<void> {
    await this.call((id) => ({ topic: "session.cancel", id, payload: { session } }));
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
    const endpoint = await readDaemonEndpoint(this.options.env);
    const ws = await dial(endpoint.wsUrl);
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

// The WebSocket to the daemon: dialing, the hello handshake, request and reply, and getting the
// connection back after it drops. What the frames mean is the client's business; this file only
// moves them.
import type { Envelope, ErrorCode } from "@neoworks-dev/neo-protocol/types";
import { busUrl, type AddressSource } from "./address.ts";
import { asError, DisconnectedError, SuiteError, TimeoutError } from "./errors.ts";
import {
  REPLY_TOPICS,
  type PayloadArgs,
  type ReplyPayload,
  type RequestTopic,
} from "./requests.ts";
import { WatchedValue } from "./watched.ts";

const PROTOCOL_VERSION = 1;
const FIRST_RETRY_DELAY_MS = 250;
const MAX_RETRY_DELAY_MS = 5000;
const NOT_CONNECTED = "not connected to the suite daemon";

/**
 * Where the connection is in its life:
 * - `connecting`: the first connect is under way (or waiting for the daemon to start).
 * - `open`: connected and introduced to the daemon.
 * - `reconnecting`: the connection dropped and the client is trying to get it back.
 * - `closed`: over for good, because `close()` was called, reconnecting is off, or the daemon
 *   refused the client.
 */
export type ConnectionState = "connecting" | "open" | "reconnecting" | "closed";

export interface ConnectionOptions {
  /** The client name sent in hello. */
  name: string;
  address: AddressSource;
  /** Try to get the connection back after it drops. */
  reconnect: boolean;
  /** Keep trying if the daemon is not running on the first connect, instead of failing. */
  waitForDaemon: boolean;
  /** How long to wait for the daemon's answer to a request, and to its welcome. */
  replyTimeoutMs: number;
  /** Runs after the daemon said welcome, on every (re)connect, before the state becomes `open`. */
  introduce: () => Promise<void>;
  /**
   * Gets every frame the daemon sends, except the welcome and errors that belong to no request.
   * A reply is passed here first and then handed to the request that is waiting for it, so
   * state built from replies is already up to date when the request resolves.
   */
  onFrame: (frame: Envelope) => void;
  /** Failures nobody is waiting for: a refused reconnect, an error frame without a request. */
  onError: (error: Error) => void;
}

interface PendingReply {
  resolve: (reply: Envelope) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface PendingWelcome {
  resolve: () => void;
  reject: (error: Error) => void;
}

export class Connection {
  readonly state: WatchedValue<ConnectionState>;
  private socket?: WebSocket;
  private nextRequestId = 1;
  private readonly pendingReplies = new Map<string, PendingReply>();
  private pendingWelcome?: PendingWelcome;
  /** Aborted by `close()`, to cut short a wait between connection attempts. */
  private readonly closing = new AbortController();

  constructor(private readonly options: ConnectionOptions) {
    this.state = new WatchedValue<ConnectionState>(
      "connection state",
      "connecting",
      options.onError,
    );
  }

  /** Makes the first connection. Rejects if the daemon cannot be reached, unless `waitForDaemon`. */
  async start(): Promise<void> {
    try {
      await this.connectWithBackoff(0, (error) => this.options.waitForDaemon && !isRefusal(error));
    } catch (error) {
      this.moveTo("closed");
      throw error;
    }
  }

  /** Sends a request and resolves with the payload of its reply. */
  async request<T extends RequestTopic>(
    topic: T,
    ...args: PayloadArgs<T>
  ): Promise<ReplyPayload<T>> {
    const reply = await this.sendRequest(topic, args[0]);
    if (reply.topic !== REPLY_TOPICS[topic]) {
      throw new Error(`unexpected ${reply.topic} reply to ${topic}`);
    }
    // REPLY_TOPICS pairs each request with the reply type that ReplyPayload reads, and the reply's
    // topic was just checked against it: this is the one place that relies on that pairing.
    const { payload } = reply as { payload?: unknown };
    return payload as ReplyPayload<T>;
  }

  /** Sends a frame that has no reply. Throws `DisconnectedError` when there is no connection. */
  send(frame: Envelope): void {
    if (!this.socket) throw new DisconnectedError(NOT_CONNECTED);
    this.socket.send(JSON.stringify(frame));
  }

  /** Closes the connection for good and fails whatever was waiting on it. */
  close(): void {
    this.closing.abort();
    // Forget the socket before closing it: closing may call onclose at once, which must not look
    // like a connection that dropped.
    const socket = this.socket;
    this.socket = undefined;
    socket?.close();
    this.failWaiting(new DisconnectedError("client closed"));
    this.moveTo("closed");
  }

  private moveTo(next: ConnectionState): void {
    // Closed is final: a connect attempt that was in flight must not bring the client back.
    if (this.state.value === next || this.state.value === "closed") return;
    this.state.set(next);
  }

  /**
   * Tries to connect, waiting `delayMs` before the first attempt and longer before each retry.
   * Gives up with the error when `mayRetry` says the error is not worth another attempt.
   */
  private async connectWithBackoff(
    delayMs: number,
    mayRetry: (error: unknown) => boolean,
  ): Promise<void> {
    let delay = delayMs;
    for (;;) {
      if (delay > 0) await sleep(delay, this.closing.signal);
      if (this.closing.signal.aborted) throw new DisconnectedError("client closed");
      try {
        await this.connectOnce();
        return;
      } catch (error) {
        if (!mayRetry(error)) throw error;
        delay = longerDelay(delay);
      }
    }
  }

  private async connectOnce(): Promise<void> {
    const socket = await dial(busUrl(await this.options.address()));
    if (this.closing.signal.aborted) {
      socket.close();
      throw new DisconnectedError("client closed");
    }
    this.socket = socket;
    socket.onmessage = (event) => this.receive(String(event.data));
    socket.onclose = () => this.lost(socket);
    try {
      await this.sayHello();
      await this.options.introduce();
    } catch (error) {
      this.socket = undefined;
      socket.close();
      throw error;
    }
    this.moveTo("open");
  }

  private sayHello(): Promise<void> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new DisconnectedError("no welcome from the daemon")),
        this.options.replyTimeoutMs,
      );
      this.pendingWelcome = {
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
        payload: { protocol: PROTOCOL_VERSION, client: this.options.name },
      });
    });
  }

  /** The socket closed on its own. */
  private lost(socket: WebSocket): void {
    if (this.socket !== socket) return;
    this.socket = undefined;
    this.failWaiting(new DisconnectedError("connection to the suite daemon closed"));
    // A connect attempt that fails is handled by the code making the attempt.
    if (this.state.value !== "open") return;
    if (!this.options.reconnect) {
      this.moveTo("closed");
      return;
    }
    this.moveTo("reconnecting");
    void this.reconnect();
  }

  private async reconnect(): Promise<void> {
    try {
      await this.connectWithBackoff(FIRST_RETRY_DELAY_MS, (error) => !isRefusal(error));
    } catch (error) {
      // The daemon answered but refused us (an invalid presence, say): trying again won't help.
      if (isRefusal(error)) this.options.onError(error);
      this.moveTo("closed");
    }
  }

  private failWaiting(reason: Error): void {
    this.pendingWelcome?.reject(reason);
    this.pendingWelcome = undefined;
    for (const [id, pending] of this.pendingReplies) {
      clearTimeout(pending.timer);
      pending.reject(reason);
      this.pendingReplies.delete(id);
    }
  }

  private sendRequest(topic: RequestTopic, payload: unknown): Promise<Envelope> {
    if (!this.socket) return Promise.reject(new DisconnectedError(NOT_CONNECTED));
    const id = String(this.nextRequestId++);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingReplies.delete(id);
        reject(
          new TimeoutError(
            `${topic} request ${id} got no reply in ${this.options.replyTimeoutMs} ms`,
          ),
        );
      }, this.options.replyTimeoutMs);
      this.pendingReplies.set(id, { resolve, reject, timer });
      try {
        this.send({ topic, id, payload } as Envelope);
      } catch (error) {
        clearTimeout(timer);
        this.pendingReplies.delete(id);
        reject(asError(error));
      }
    });
  }

  private receive(text: string): void {
    let frame: Envelope;
    try {
      frame = JSON.parse(text);
    } catch {
      return;
    }
    if (typeof frame !== "object" || frame === null) return;
    if (frame.topic === "bus.welcome") {
      this.pendingWelcome?.resolve();
      this.pendingWelcome = undefined;
      return;
    }
    if (frame.topic === "bus.error" && frame.id === undefined) {
      this.rejectWithoutRequest(frame.payload.code, frame.payload.message);
      return;
    }
    this.options.onFrame(frame);
    if ("id" in frame && typeof frame.id === "string") this.answer(frame.id, frame);
  }

  /** An error that answers no request: a failed hello, or something the daemon could not parse. */
  private rejectWithoutRequest(code: ErrorCode, message: string): void {
    const error = new SuiteError(code, message);
    if (this.pendingWelcome) {
      this.pendingWelcome.reject(error);
      this.pendingWelcome = undefined;
    } else this.options.onError(error);
  }

  /** Hands a frame carrying a request's id to that request. */
  private answer(id: string, frame: Envelope): void {
    const pending = this.pendingReplies.get(id);
    if (!pending) return;
    this.pendingReplies.delete(id);
    clearTimeout(pending.timer);
    if (frame.topic === "bus.error") {
      pending.reject(new SuiteError(frame.payload.code, frame.payload.message));
    } else pending.resolve(frame);
  }
}

/** The daemon answered and said no. Only a missing daemon is worth retrying. */
function isRefusal(error: unknown): error is SuiteError {
  return error instanceof SuiteError;
}

function longerDelay(delayMs: number): number {
  if (delayMs === 0) return FIRST_RETRY_DELAY_MS;
  return Math.min(delayMs * 2, MAX_RETRY_DELAY_MS);
}

/** Waits `ms`, or less if `signal` aborts. */
function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
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

/** Opens a WebSocket; rejects if it can't be established. */
function dial(url: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    socket.onopen = () => resolve(socket);
    socket.onerror = () => reject(new DisconnectedError("could not reach the suite daemon"));
    socket.onclose = () => reject(new DisconnectedError("could not reach the suite daemon"));
  });
}

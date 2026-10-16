// Following sessions: their event log and their resolved views. Both work the same way. A
// subscription asks the daemon to start, takes the frames that belong to it, and is asked again
// whenever the connection comes back, so it carries on where it left off.
import type {
  Envelope,
  SessionEvent,
  SessionEventRecord,
  SessionInfo,
  ViewPatch,
  ViewRef,
  ViewSnapshot,
} from "@suite/protocol/types";
import type { Connection } from "./connection.ts";
import { callGuarded, reportUnlessDisconnected } from "./errors.ts";
import type { Unsubscribe } from "./watched.ts";

export interface SubscribeOptions {
  /** First seq to deliver (inclusive); defaults to 1, the whole history. */
  fromSeq?: number;
  /** Each event once, in seq order, across reconnects. */
  onEvent: (event: SessionEvent, record: SessionEventRecord) => void;
  /** The replay is over and events are live. Fires again after each reconnect. */
  onLive?: (session: SessionInfo) => void;
  /** The daemon refused the subscription (an unknown session, say). */
  onError?: (error: Error) => void;
}

export interface ViewSubscribeOptions {
  /**
   * Only the work of this agent in the session (a subagent, or a guest the user addressed),
   * instead of the session's own view.
   */
  agent?: string;
  /**
   * The whole view as of `snapshot.seq`; replaces whatever you had. Fires again after each
   * reconnect, because the daemon answers a new subscription with a fresh snapshot.
   */
  onSnapshot: (snapshot: ViewSnapshot) => void;
  /** A change after the latest snapshot, in order. Never delivered before a snapshot. */
  onPatch: (patch: ViewPatch) => void;
  /** The daemon refused the subscription (an unknown session, say). */
  onError?: (error: Error) => void;
}

/** What the subscriptions need from the client. */
interface Wire {
  connection: Connection;
  /** Receives callback errors and failures of subscriptions that have no `onError` of their own. */
  onError: (error: Error) => void;
}

interface Subscription {
  /**
   * Subscriptions with the same key are one subscription to the daemon, which keeps only one per
   * connection; it is told to stop when the last of them goes.
   */
  readonly key: string;
  /** Asks the daemon to start sending. Failures go to the subscription's error handler. */
  start(): void;
  /** Asks the daemon to stop sending. */
  stop(): Promise<void>;
  /** Looks at a frame from the daemon and takes it if it belongs here. */
  take(frame: Envelope): void;
}

class EventSubscription implements Subscription {
  readonly key: string;
  /** The next seq to deliver; also where a resume starts. */
  private nextSeq: number;

  constructor(
    private readonly session: string,
    private readonly options: SubscribeOptions,
    private readonly wire: Wire,
  ) {
    this.key = `events ${session}`;
    this.nextSeq = options.fromSeq ?? 1;
  }

  start(): void {
    const request = { session: this.session, fromSeq: this.nextSeq };
    this.wire.connection
      .request("session.subscribe", request)
      .then((reply) => this.options.onLive?.(reply.session))
      .catch((thrown: unknown) => {
        reportUnlessDisconnected(thrown, this.options.onError ?? this.wire.onError);
      });
  }

  async stop(): Promise<void> {
    await this.wire.connection.request("session.unsubscribe", { session: this.session });
  }

  take(frame: Envelope): void {
    if (frame.topic !== "session.events") return;
    const record = frame.payload;
    // Another subscription to the same session can make the daemon replay what we already have.
    if (record.session !== this.session || record.seq < this.nextSeq) return;
    this.nextSeq = record.seq + 1;
    callGuarded(this.wire.onError, () => this.options.onEvent(record.event, record));
  }
}

class ViewSubscription implements Subscription {
  readonly key: string;
  private readonly ref: ViewRef;
  /** True from asking until the snapshot arrives: patches in between are for a state we lack. */
  private awaitingSnapshot = true;

  constructor(
    private readonly session: string,
    private readonly options: ViewSubscribeOptions,
    private readonly wire: Wire,
  ) {
    this.ref = { session, agent: options.agent };
    this.key = `view ${session} ${options.agent ?? ""}`;
  }

  start(): void {
    this.awaitingSnapshot = true;
    this.wire.connection
      .request("view.subscribe", this.ref)
      .then((snapshot) => {
        this.awaitingSnapshot = false;
        callGuarded(this.wire.onError, () => this.options.onSnapshot(snapshot));
      })
      .catch((thrown: unknown) => {
        reportUnlessDisconnected(thrown, this.options.onError ?? this.wire.onError);
      });
  }

  async stop(): Promise<void> {
    await this.wire.connection.request("view.unsubscribe", this.ref);
  }

  take(frame: Envelope): void {
    if (frame.topic !== "view.patch" || this.awaitingSnapshot) return;
    const patch = frame.payload;
    if (patch.session !== this.session || patch.agent !== this.options.agent) return;
    callGuarded(this.wire.onError, () => this.options.onPatch(patch));
  }
}

/** All live subscriptions of a client. */
export class Subscriptions {
  private readonly active = new Set<Subscription>();

  constructor(private readonly wire: Wire) {}

  /** Replays the session's events from `options.fromSeq`, then follows it live. */
  followEvents(session: string, options: SubscribeOptions): Unsubscribe {
    return this.add(new EventSubscription(session, options, this.wire));
  }

  /** Delivers a snapshot of the view, then its patches. */
  followView(session: string, options: ViewSubscribeOptions): Unsubscribe {
    return this.add(new ViewSubscription(session, options, this.wire));
  }

  /** Asks the daemon again for everything, after the connection came back. */
  resume(): void {
    for (const subscription of this.active) subscription.start();
  }

  /** Gives a frame from the daemon to the subscriptions it belongs to. */
  take(frame: Envelope): void {
    // A copy: a callback may unsubscribe while we go through the set.
    for (const subscription of [...this.active]) subscription.take(frame);
  }

  /** Forgets every subscription without telling the daemon (the connection is going away). */
  clear(): void {
    this.active.clear();
  }

  /** Subscribing while disconnected is fine: it starts when the connection opens. */
  private add(subscription: Subscription): Unsubscribe {
    this.active.add(subscription);
    if (this.isOpen()) subscription.start();
    return () => this.remove(subscription);
  }

  private remove(subscription: Subscription): void {
    if (!this.active.delete(subscription)) return;
    const stillWanted = [...this.active].some((other) => other.key === subscription.key);
    if (this.isOpen() && !stillWanted) subscription.stop().catch(() => {});
  }

  private isOpen(): boolean {
    return this.wire.connection.state.value === "open";
  }
}

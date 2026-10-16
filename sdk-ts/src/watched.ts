import { callGuarded, TimeoutError } from "./errors.ts";

/** Call it to stop listening. Calling it again does nothing. */
export type Unsubscribe = () => void;

const DEFAULT_WAIT_MS = 10_000;

/**
 * A value that changes over time, with a way to be told when it does. The client uses it for
 * everything the daemon keeps current for you: focus, theme, connectors, sessions and the
 * connection state.
 */
export interface Watched<T> {
  /** The latest value. */
  readonly value: T;

  /**
   * Calls `callback` with every new value from now on (not with the current one). Stop with the
   * returned function. If `callback` throws, the error goes to the client's `onError`; other
   * callbacks still run.
   */
  on(callback: (value: T) => void): Unsubscribe;

  /**
   * Resolves with the first value that is not `undefined`: the current one, or the next one to
   * arrive. Rejects with `TimeoutError` after `timeoutMs` (default 10 seconds).
   */
  waitFor(timeoutMs?: number): Promise<Exclude<T, undefined>>;

  /**
   * Resolves with the first value `accept` approves: the current one, or the next one that
   * arrives. Rejects with `TimeoutError` after `timeoutMs`.
   */
  waitFor(timeoutMs: number, accept: (value: T) => boolean): Promise<T>;
}

/** A set of callbacks that are called together, each shielded from the others' errors. */
export class Callbacks<T> {
  private readonly entries = new Set<(value: T) => void>();

  constructor(private readonly onError: (error: Error) => void) {}

  add(callback: (value: T) => void): Unsubscribe {
    this.entries.add(callback);
    return () => {
      this.entries.delete(callback);
    };
  }

  callAll(value: T): void {
    // A copy, so a callback may unsubscribe itself (or others) while we go through them.
    for (const callback of [...this.entries]) {
      callGuarded(this.onError, () => callback(value));
    }
  }
}

function isPresent(value: unknown): boolean {
  return value !== undefined;
}

/** The writable side of a `Watched`, kept inside the client. */
export class WatchedValue<T> implements Watched<T> {
  private latest: T;
  private readonly callbacks: Callbacks<T>;

  /** `what` names the value in timeout messages, e.g. "theme". */
  constructor(
    private readonly what: string,
    initial: T,
    onError: (error: Error) => void,
  ) {
    this.latest = initial;
    this.callbacks = new Callbacks(onError);
  }

  get value(): T {
    return this.latest;
  }

  /** Makes `value` the latest and tells everyone listening, even if it equals the old one. */
  set(value: T): void {
    this.latest = value;
    this.callbacks.callAll(value);
  }

  on(callback: (value: T) => void): Unsubscribe {
    return this.callbacks.add(callback);
  }

  waitFor(timeoutMs?: number): Promise<Exclude<T, undefined>>;
  waitFor(timeoutMs: number, accept: (value: T) => boolean): Promise<T>;
  waitFor(timeoutMs = DEFAULT_WAIT_MS, accept: (value: T) => boolean = isPresent): Promise<T> {
    if (accept(this.latest)) return Promise.resolve(this.latest);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        stopListening();
        reject(new TimeoutError(`timed out after ${timeoutMs} ms waiting for ${this.what}`));
      }, timeoutMs);
      const stopListening = this.on((value) => {
        if (!accept(value)) return;
        clearTimeout(timer);
        stopListening();
        resolve(value);
      });
    });
  }
}

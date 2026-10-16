import type { ErrorCode } from "@suite/protocol/types";

/** The daemon answered a request with an error. `code` says why (for example `not_found`). */
export class SuiteError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "SuiteError";
  }
}

/** There is no live connection to the daemon: never connected, dropped, or closed. */
export class DisconnectedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DisconnectedError";
  }
}

/** The daemon did not answer a request in time, or a `waitFor` ran out of time. */
export class TimeoutError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TimeoutError";
  }
}

/** Wraps whatever was thrown in an `Error`, so handlers can rely on one type. */
export function asError(thrown: unknown): Error {
  if (thrown instanceof Error) return thrown;
  return new Error(String(thrown));
}

/**
 * Runs `callback` and passes anything it throws to `onError`: a faulty listener must not stop
 * the others, or the connection code that called it.
 */
export function callGuarded(onError: (error: Error) => void, callback: () => void): void {
  try {
    callback();
  } catch (thrown) {
    onError(asError(thrown));
  }
}

/**
 * For requests nobody awaits: a dropped connection is not worth reporting (the client reconnects
 * and asks again), anything else goes to `onError`.
 */
export function reportUnlessDisconnected(thrown: unknown, onError: (error: Error) => void): void {
  if (thrown instanceof DisconnectedError) return;
  onError(asError(thrown));
}

/** Minimal logging interface; the daemon passes its own implementation. */
export interface Logger {
  warn(message: string): void;
  error(message: string): void;
}

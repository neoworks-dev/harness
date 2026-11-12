// A button with a deadline (a question that answers itself when nobody does) counts down to it.

/** `2:41`: what is left of `ms`, rounded up to the second, never below 0:00. */
export function timeLeft(ms: number): string {
  const s = Math.ceil(Math.max(0, ms) / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The label with the time left until `deadline` (unix ms), or the label alone without one. */
export function withTimeLeft(label: string, deadline: number | undefined, now: number): string {
  return deadline === undefined ? label : `${label} · ${timeLeft(deadline - now)}`;
}

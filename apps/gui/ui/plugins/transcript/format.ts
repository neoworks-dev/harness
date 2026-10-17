// How the transcript writes durations and times. Plain TypeScript, tested without a window.

/** A duration as the "Working for" and "Worked for" rows write it: `12s`, `3m 43s`, `1h 5m`. */
export function duration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** The time of day of a Unix time in milliseconds, as `14:05`, in the local zone. */
export function clockTime(ts: number): string {
  const date = new Date(ts);
  const two = (n: number): string => String(n).padStart(2, "0");
  return `${two(date.getHours())}:${two(date.getMinutes())}`;
}

/** The last `cap` lines of `text`, and how many came before them: the end of an output is what matters. */
export function tailLines(text: string, cap: number): { text: string; hidden: number } {
  const lines = text.split("\n");
  if (lines.length <= cap + 1) return { text, hidden: 0 };
  return { text: lines.slice(-cap).join("\n"), hidden: lines.length - cap };
}

/** `1 file`, `3 files`. */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

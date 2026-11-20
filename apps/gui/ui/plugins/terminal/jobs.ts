// Background commands (crates/jobs): a command Claude Code runs in the background ends its call at
// once, naming the file its output goes to; the file is read here as it grows, and its last line
// says how the command ended. As term reads them (apps/term/src/widgets/terminal.rs). Plain
// functions, tested without a window.
import { OUTPUT_LINES, plainOutput } from "./runs.ts";

/** What a read of the file found (the host's `job_read`). */
export interface Chunk {
  text: string;
  offset: number;
  missing: boolean;
}

export interface JobState {
  path: string;
  /** Where the next read starts, and the start of a line not ended yet. */
  offset: number;
  partial: string;
  /** The last `OUTPUT_LINES` lines, colour codes taken out. */
  lines: readonly string[];
  /** When its command started (ms), as the session's events say; when it ended, and how. */
  startedAt: number;
  /** When this window first saw it (ms): how long a missing file is waited for counts from here. */
  seenAt: number;
  endedAt?: number;
  exit?: number;
  /** Read while still running: one replayed with the session's history is found ended at once. */
  seenRunning: boolean;
  /** The file isn't there (cleaned up, or never written). */
  gone: boolean;
  /** The file ends with `[killed]`: it was stopped, and is dropped. */
  killed: boolean;
}

/** How long a missing file is waited for before the job counts as gone. */
const MISSING_MS = 10_000;
/** How long an ended job stays in the list under the input. */
export const LINGER_MS = 5000;

/** The output file a background command's result names: `… Output is being written to: <path>.` */
export function backgroundOutput(text: string): string | undefined {
  if (!text.includes("running in background")) return undefined;
  const rest = text.split("Output is being written to: ")[1];
  const path = rest?.trim().split(/\s+/)[0]?.replace(/\.+$/, "");
  return path?.startsWith("/") ? path : undefined;
}

/** A job first seen at `nowMs`, whose command started at `startedAt` (a session opened late sees it running for a while). */
export function newJob(path: string, nowMs: number, startedAt = nowMs): JobState {
  return {
    path,
    offset: 0,
    partial: "",
    lines: [],
    startedAt,
    seenAt: nowMs,
    seenRunning: false,
    gone: false,
    killed: false,
  };
}

/** Whether the job may still write: it is read again. */
export function isActive(job: JobState): boolean {
  return job.exit === undefined && !job.gone && !job.killed;
}

/** `[exited with code 0]`: the code. */
function exitCode(line: string): number | undefined {
  const match = /^\[exited with code (-?\d+)\]$/.exec(line.trim());
  return match === null ? undefined : Number(match[1]);
}

/** The job after a read of its file at `nowMs`. */
export function applyChunk(job: JobState, chunk: Chunk, nowMs: number): JobState {
  if (chunk.missing) {
    const gone = job.offset > 0 || nowMs - job.seenAt > MISSING_MS;
    return gone ? { ...job, gone } : job;
  }
  const partial = job.partial + chunk.text;
  const end = partial.lastIndexOf("\n");
  // No line ended: it runs on.
  if (end < 0) return { ...job, offset: chunk.offset, partial, seenRunning: true };
  const lines = plainOutput(partial.slice(0, end)).split("\n");
  const next: JobState = { ...job, offset: chunk.offset, partial: partial.slice(end + 1) };
  if (lines.some((line) => line.trim() === "[killed]")) return { ...next, killed: true };
  const exit = exitCode(lines.at(-1) ?? "");
  if (exit !== undefined) {
    lines.pop();
    if (lines.at(-1) === "") lines.pop();
    next.exit = exit;
    next.endedAt = nowMs;
  } else {
    next.seenRunning = true;
  }
  next.lines = [...job.lines, ...lines].slice(-OUTPUT_LINES);
  return next;
}

/** Whether the job belongs in the list under the input: found running, and not ended a while ago. */
export function isListed(job: JobState, nowMs: number): boolean {
  if (job.gone || job.killed || !job.seenRunning) return false;
  return job.endedAt === undefined || nowMs - job.endedAt < LINGER_MS;
}

/** Its last line of output that says something. */
export function lastLine(job: JobState): string | undefined {
  for (let i = job.lines.length - 1; i >= 0; i -= 1) {
    const line = job.lines[i];
    if (line !== undefined && line.trim() !== "") return line;
  }
  return undefined;
}

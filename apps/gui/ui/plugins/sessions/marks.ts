// What the user marks sessions with in the sidebar: pinned ones, and settled ones (put away until
// they do something). The daemon knows neither, so the window keeps them (marks-store.ts writes
// them to disk). Plain functions on immutable data.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";

export interface Marks {
  /** Pinned sessions by id, with when each was pinned (Unix ms). */
  pinned: Readonly<Record<string, number>>;
  /** Settled sessions by id, with the session's `lastActivity` when it was settled. */
  settled: Readonly<Record<string, number>>;
}

export const NO_MARKS: Marks = { pinned: {}, settled: {} };

function without(record: Readonly<Record<string, number>>, id: string): Record<string, number> {
  const { [id]: _gone, ...rest } = record;
  return rest;
}

export function isPinned(marks: Marks, id: string): boolean {
  return marks.pinned[id] !== undefined;
}

/**
 * Whether a session is put away: it was settled and has done nothing since. Any later activity
 * (`lastActivity` past what it was when settled) brings it back without anyone un-settling it.
 */
export function isSettled(marks: Marks, info: SessionInfo): boolean {
  const at = marks.settled[info.id];
  return at !== undefined && info.lastActivity <= at;
}

/** Pins a session. A pinned session is not settled. */
export function pin(marks: Marks, id: string, nowMs: number): Marks {
  return { pinned: { ...marks.pinned, [id]: nowMs }, settled: without(marks.settled, id) };
}

export function unpin(marks: Marks, id: string): Marks {
  return { pinned: without(marks.pinned, id), settled: marks.settled };
}

/** Settles a session as it is now. A settled session is not pinned. */
export function settle(marks: Marks, info: SessionInfo): Marks {
  return {
    pinned: without(marks.pinned, info.id),
    settled: { ...marks.settled, [info.id]: info.lastActivity },
  };
}

export function unsettle(marks: Marks, id: string): Marks {
  return { pinned: marks.pinned, settled: without(marks.settled, id) };
}

/** A session and every session delegated to from it, or from those, in `all`. */
export function familyOf(info: SessionInfo, all: readonly SessionInfo[]): SessionInfo[] {
  const out = [info];
  const seen = new Set([info.id]);
  for (let at = 0; at < out.length; at += 1) {
    for (const other of all) {
      if (other.parent === out[at]!.id && !seen.has(other.id)) {
        seen.add(other.id);
        out.push(other);
      }
    }
  }
  return out;
}

/** Settles a session with its subagents' sessions, or (`settled`) brings them all back. */
export function toggleSettled(
  marks: Marks,
  info: SessionInfo,
  all: readonly SessionInfo[],
  settled: boolean,
): Marks {
  return familyOf(info, all).reduce(
    (next, member) => (settled ? unsettle(next, member.id) : settle(next, member)),
    marks,
  );
}

/** Both sets of marks, `over` winning where they mark the same session. */
export function mergeMarks(base: Marks, over: Marks): Marks {
  const pinned = { ...base.pinned, ...over.pinned };
  const settled = { ...base.settled, ...over.settled };
  for (const id of Object.keys(over.pinned)) delete settled[id];
  for (const id of Object.keys(over.settled)) delete pinned[id];
  return { pinned, settled };
}

function numbers(value: unknown): Record<string, number> {
  const out: Record<string, number> = {};
  if (typeof value !== "object" || value === null) return out;
  for (const [id, at] of Object.entries(value)) {
    if (typeof at === "number" && Number.isFinite(at)) out[id] = at;
  }
  return out;
}

/** The marks in a saved file; none when it is empty or not what `serializeMarks` writes. */
export function parseMarks(text: string): Marks {
  try {
    const data: unknown = JSON.parse(text);
    if (typeof data !== "object" || data === null) return NO_MARKS;
    const record = data as Record<string, unknown>;
    return { pinned: numbers(record.pinned), settled: numbers(record.settled) };
  } catch {
    return NO_MARKS;
  }
}

export function serializeMarks(marks: Marks): string {
  return `${JSON.stringify({ pinned: marks.pinned, settled: marks.settled }, null, 2)}\n`;
}

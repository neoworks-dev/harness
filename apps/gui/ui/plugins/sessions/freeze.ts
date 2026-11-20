// The sidebar's order holds still: sessions are arranged by a snapshot of what they were doing,
// taken again only when a session is added or every REARRANGE_MS, so a session that works does
// not move the list under the pointer. Rows still draw what each session does now. Subagents'
// sessions are the exception: they come and go as their work does. Plain functions.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import type { Arranged, Line } from "./arrange.ts";

/** The longest the order holds still. */
export const REARRANGE_MS = 30 * 60_000;

export interface Frozen {
  /** When the snapshot was taken (Unix ms). */
  at: number;
  byId: ReadonlyMap<string, SessionInfo>;
}

function snapshot(live: readonly SessionInfo[], nowMs: number): Frozen {
  return { at: nowMs, byId: new Map(live.map((info) => [info.id, info])) };
}

/** The snapshot to arrange by: taken again when a session is new to it, or it is too old. */
export function refreeze(
  frozen: Frozen | undefined,
  live: readonly SessionInfo[],
  nowMs: number,
): Frozen {
  if (frozen === undefined || nowMs - frozen.at >= REARRANGE_MS) return snapshot(live, nowMs);
  return live.some((info) => !frozen.byId.has(info.id)) ? snapshot(live, nowMs) : frozen;
}

/** The sessions to arrange: each as the snapshot saw it, a subagent's as it is now. */
export function basis(frozen: Frozen, live: readonly SessionInfo[]): SessionInfo[] {
  return live.map((info) =>
    info.parent !== undefined ? info : (frozen.byId.get(info.id) ?? info),
  );
}

/** The arranged lines with each session as it is now, for the rows to draw. */
export function withLive(arranged: Arranged, live: readonly SessionInfo[]): Arranged {
  const byId = new Map(live.map((info) => [info.id, info]));
  const lines = arranged.lines.map((line): Line => {
    if (line.kind !== "session") return line;
    const info = byId.get(line.info.id);
    return info === undefined || info === line.info ? line : { ...line, info };
  });
  return { ...arranged, lines };
}

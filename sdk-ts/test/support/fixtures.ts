import type { SessionInfo, ViewItem } from "@neoworks-dev/neo-protocol/types";

/** A view item holding one paragraph. */
export function viewItem(ref: string, text: string): ViewItem {
  return {
    ref,
    kind: "agent",
    component: { type: "text", blocks: [{ type: "paragraph", spans: [{ text }] }] },
  };
}

export function sessionInfo(id: string, changes: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    cwd: "/tmp",
    createdAt: 1,
    lastActivity: 1,
    lastSeq: 0,
    state: "idle",
    queued: 0,
    ...changes,
  };
}

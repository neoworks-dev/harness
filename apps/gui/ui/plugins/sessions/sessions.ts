// The sessions sidebar, as plain functions: where a session works, how the list is grouped and
// ordered, how old a session is and which ones finished while another was shown. Ported from the
// terminal app (apps/term/src/sessions.rs).
import type { SessionInfo } from "@suite/protocol/types";

/** Where a session works: a repository (or, outside one, the directory itself) and the worktree. */
export interface Place {
  repo: string;
  /** Absent for the main checkout. */
  worktree?: string;
}

/** The last path segment. */
export function lastSegment(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1);
}

const WORKTREES = "/.worktrees/";

/**
 * The place a directory has by the `.worktrees/<name>` convention, which needs no disk access.
 * Anything else is for the host to find (`places` command), which reads the `.git` files.
 */
export function conventionPlace(cwd: string): Place | undefined {
  const dir = cwd.replace(/\/+$/, "");
  const at = dir.indexOf(WORKTREES);
  if (at < 0) return undefined;
  const worktree = dir.slice(at + WORKTREES.length).split("/")[0] ?? "";
  if (worktree === "") return undefined;
  return { repo: dir.slice(0, at), worktree };
}

/** Where `cwd` works: what the host found, else the convention, else the directory itself. */
export function placeOf(cwd: string, found: Readonly<Record<string, Place>>): Place {
  const resolved = found[cwd];
  if (resolved) return resolved;
  const byConvention = conventionPlace(cwd);
  if (byConvention) return byConvention;
  return { repo: cwd.replace(/\/+$/, "") };
}

/** One line-group of the drawn list. */
export type Entry =
  | { kind: "cluster"; key: string; name: string }
  | { kind: "group"; key: string; name: string }
  | { kind: "session"; key: string; info: SessionInfo; child: boolean };

export interface Grouped {
  /** Headers and sessions, in drawn order. */
  entries: readonly Entry[];
  /** The sessions' ids in drawn order: what the arrow keys walk. */
  order: readonly string[];
  /** Positions in `order` where a group of sessions starts. */
  groupStarts: readonly number[];
}

function compareText(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

function newestFirst(a: SessionInfo, b: SessionInfo): number {
  return b.lastActivity - a.lastActivity || compareText(a.id, b.id);
}

/** A group's sessions in drawn order: each right after its parent (as a child), the others by recency. */
function family(group: readonly SessionInfo[]): { info: SessionInfo; child: boolean }[] {
  const ids = new Set(group.map((s) => s.id));
  const hasParentHere = (s: SessionInfo): boolean => s.parent !== undefined && ids.has(s.parent);
  const out: { info: SessionInfo; child: boolean }[] = [];
  for (const info of group.filter((s) => !hasParentHere(s))) {
    // A session that delegated here but whose parent is elsewhere still wears the mark.
    out.push({ info, child: info.parent !== undefined });
    for (const other of group) if (other.parent === info.id) out.push({ info: other, child: true });
  }
  return out;
}

interface Cluster {
  repo: string;
  latest: number;
  main: SessionInfo[];
  worktrees: Map<string, { latest: number; sessions: SessionInfo[] }>;
}

/**
 * Groups the sessions: repositories by their latest activity, in each the main checkout and then
 * its worktrees by theirs, in each the sessions by theirs with children after their parent.
 */
export function groupSessions(
  sessions: readonly SessionInfo[],
  found: Readonly<Record<string, Place>>,
): Grouped {
  const clusters = new Map<string, Cluster>();
  for (const info of [...sessions].sort(newestFirst)) {
    const place = placeOf(info.cwd, found);
    let cluster = clusters.get(place.repo);
    if (!cluster) {
      cluster = { repo: place.repo, latest: 0, main: [], worktrees: new Map() };
      clusters.set(place.repo, cluster);
    }
    cluster.latest = Math.max(cluster.latest, info.lastActivity);
    if (place.worktree === undefined) {
      cluster.main.push(info);
      continue;
    }
    const tree = cluster.worktrees.get(place.worktree) ?? { latest: 0, sessions: [] };
    tree.latest = Math.max(tree.latest, info.lastActivity);
    tree.sessions.push(info);
    cluster.worktrees.set(place.worktree, tree);
  }

  const entries: Entry[] = [];
  const order: string[] = [];
  const groupStarts: number[] = [];
  const put = (group: readonly SessionInfo[]): void => {
    if (group.length === 0) return;
    groupStarts.push(order.length);
    for (const { info, child } of family(group)) {
      order.push(info.id);
      entries.push({ kind: "session", key: info.id, info, child });
    }
  };
  const ordered = [...clusters.values()].sort(
    (a, b) => b.latest - a.latest || compareText(a.repo, b.repo),
  );
  for (const cluster of ordered) {
    entries.push({ kind: "cluster", key: `repo ${cluster.repo}`, name: lastSegment(cluster.repo) });
    put(cluster.main);
    const trees = [...cluster.worktrees.entries()].sort(
      (a, b) => b[1].latest - a[1].latest || compareText(a[0], b[0]),
    );
    for (const [name, tree] of trees) {
      entries.push({ kind: "group", key: `tree ${cluster.repo} ${name}`, name });
      put(tree.sessions);
    }
  }
  return { entries, order, groupStarts };
}

/** The session's title on one line, else "session " and the start of its id. */
export function sessionTitle(info: SessionInfo): string {
  const title = info.title?.trim();
  if (title) return title.split(/\s+/).join(" ");
  return `session ${info.id.slice(0, 8)}`;
}

/** The latest thing said, on one line. */
export function previewOf(info: SessionInfo): string {
  return (info.preview ?? "").split(/\s+/).filter(Boolean).join(" ");
}

/** How long ago, short: now, 5m, 3h, 2d. */
export function age(thenMs: number, nowMs: number): string {
  const secs = Math.floor(Math.max(0, nowMs - thenMs) / 1000);
  if (secs < 60) return "now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86_400)}d`;
}

/** The lines a session's edits changed; a zero side is left out. */
export function changedLines(info: SessionInfo): { added: number; removed: number } {
  return { added: Math.max(0, info.added ?? 0), removed: Math.max(0, info.removed ?? 0) };
}

/**
 * The sessions that finished a turn while another one was shown, and have not been looked at
 * since. A session that was running and is not any more is added, unless it is the one shown;
 * showing a session, or its going away, takes it off.
 */
export function finishedAfter(
  finished: ReadonlySet<string>,
  previous: readonly SessionInfo[],
  next: readonly SessionInfo[],
  shown: string | undefined,
): ReadonlySet<string> {
  const wasRunning = new Set(previous.filter((s) => s.state === "running").map((s) => s.id));
  const present = new Set(next.map((s) => s.id));
  const out = new Set([...finished].filter((id) => present.has(id) && id !== shown));
  for (const info of next) {
    if (wasRunning.has(info.id) && info.state !== "running" && info.id !== shown) out.add(info.id);
  }
  const same = out.size === finished.size && [...out].every((id) => finished.has(id));
  return same ? finished : out;
}

/** What a session's row says in place of its age. */
export type Status =
  | { kind: "working"; since: number | undefined }
  | { kind: "done" }
  | { kind: "idle" };

/** The ladder: working, then finished unseen, else idle. `since` is when it was seen to start. */
export function statusOf(
  info: SessionInfo,
  finished: boolean,
  since: Readonly<Record<string, number>>,
): Status {
  if (info.state === "running") return { kind: "working", since: since[info.id] };
  return finished ? { kind: "done" } : { kind: "idle" };
}

/** How long a turn has run, short: 12s, 3m 05s, 1h 02m. */
export function elapsed(fromMs: number, nowMs: number): string {
  const secs = Math.floor(Math.max(0, nowMs - fromMs) / 1000);
  const two = (n: number): string => String(n).padStart(2, "0");
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${two(secs % 60)}s`;
  return `${Math.floor(secs / 3600)}h ${two(Math.floor((secs % 3600) / 60))}m`;
}

/**
 * When each running session was first seen running: kept while it runs, set to `nowMs` when it is
 * first seen, dropped when it stops. The daemon does not say when a turn began, so a session that
 * was already running when the window opened counts from then.
 */
export function runningSinceAfter(
  since: Readonly<Record<string, number>>,
  next: readonly SessionInfo[],
  nowMs: number,
): Readonly<Record<string, number>> {
  const out: Record<string, number> = {};
  for (const info of next) if (info.state === "running") out[info.id] = since[info.id] ?? nowMs;
  const same =
    Object.keys(out).length === Object.keys(since).length &&
    Object.entries(out).every(([id, at]) => since[id] === at);
  return same ? since : out;
}

/** The id `delta` rows from `from` in drawn order, stopping at the ends. */
export function step(
  order: readonly string[],
  from: string | undefined,
  delta: number,
): string | undefined {
  if (order.length === 0) return undefined;
  const at = from === undefined ? 0 : Math.max(0, order.indexOf(from));
  return order[Math.min(order.length - 1, Math.max(0, at + delta))];
}

/** The first session of the previous (`-1`) or next (`1`) group. */
export function jumpGroup(
  grouped: Pick<Grouped, "order" | "groupStarts">,
  from: string | undefined,
  delta: number,
): string | undefined {
  const at = Math.max(0, from === undefined ? 0 : grouped.order.indexOf(from));
  let current = 0;
  grouped.groupStarts.forEach((start, index) => {
    if (start <= at) current = index;
  });
  const target = Math.min(grouped.groupStarts.length - 1, Math.max(0, current + delta));
  const start = grouped.groupStarts[target];
  return start === undefined ? undefined : grouped.order[start];
}

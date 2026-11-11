// What the sidebar draws, top to bottom: the pinned sessions, the rest grouped as `groupSessions`
// does, then the settled ones in a section of their own; or the results of a search. Plus the
// projects the sessions belong to, for the filter. Plain functions.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { isPinned, isSettled, type Marks } from "./marks.ts";
import type { Match } from "./search.ts";
import { groupSessions, lastSegment, placeOf, type Place } from "./sessions.ts";

/** Settled sessions listed before "Show more". */
export const SETTLED_SHOWN = 10;
/** Sessions of a repository or worktree listed before "N more". */
export const GROUP_SHOWN = 3;

/** One line of the drawn list. */
export type Line =
  /** A heading: "Pinned", a repository, or (`sub`) a worktree of the one above. */
  | { kind: "label"; key: string; text: string; sub?: boolean }
  | {
      kind: "session";
      key: string;
      info: SessionInfo;
      /** Delegated to by another session in the same list. */
      child: boolean;
      /** A settled session: a slim row. */
      slim: boolean;
      /** What a search found it by. */
      field?: Match["field"];
    }
  | { kind: "settled"; key: string; count: number; open: boolean }
  /** The sessions a group (a repository's or worktree's key, or "settled") leaves out. */
  | { kind: "more"; key: string; count: number; group: string };

export interface Arranged {
  lines: readonly Line[];
  /** The sessions' ids in drawn order: what the arrow keys walk. */
  order: readonly string[];
  /** Positions in `order` where a group of sessions starts. */
  groupStarts: readonly number[];
}

export interface ShowSettled {
  open: boolean;
  /** Past the first `SETTLED_SHOWN`. */
  all: boolean;
  /** The groups opened past their first `GROUP_SHOWN`. */
  groups: ReadonlySet<string>;
  /** Sessions that never fold into "N more": the shown one, one that works or waits. */
  keep: (info: SessionInfo) => boolean;
}

function newestFirst(a: SessionInfo, b: SessionInfo): number {
  return b.lastActivity - a.lastActivity || (a.id < b.id ? -1 : 1);
}

/**
 * The list without a search: the pinned sessions (the one pinned last first), the others by
 * project and worktree as `groupSessions` orders them, each under its heading and three of each
 * until opened, then "Settled (n)", which lists nothing
 * until it is opened, and then ten of its sessions, newest first, and a line for the rest.
 */
export function arrange(
  sessions: readonly SessionInfo[],
  found: Readonly<Record<string, Place>>,
  marks: Marks,
  settledShown: ShowSettled,
): Arranged {
  const pinned = sessions
    .filter((info) => isPinned(marks, info.id))
    .sort((a, b) => (marks.pinned[b.id] ?? 0) - (marks.pinned[a.id] ?? 0));
  const settled = sessions
    .filter((info) => !isPinned(marks, info.id) && isSettled(marks, info))
    .sort(newestFirst);
  const apart = new Set([...pinned, ...settled].map((info) => info.id));
  const others = sessions.filter((info) => !apart.has(info.id));

  const lines: Line[] = [];
  const order: string[] = [];
  const groupStarts: number[] = [];
  const add = (info: SessionInfo, child: boolean, slim: boolean): void => {
    order.push(info.id);
    lines.push({ kind: "session", key: info.id, info, child, slim });
  };

  if (pinned.length > 0) {
    lines.push({ kind: "label", key: "pinned", text: "Pinned" });
    groupStarts.push(order.length);
    for (const info of pinned) add(info, false, false);
  }

  // Each repository under its name and each worktree under its own, as the terminal draws them:
  // the first GROUP_SHOWN of a group, then a line for the rest until it is opened. A session
  // that works, waits to be looked at or is shown never folds; a child goes with its parent.
  let group = "";
  let starting = false;
  let shown = 0;
  let hidden = 0;
  let parentHidden = false;
  const close = (): void => {
    if (hidden > 0) lines.push({ kind: "more", key: `more ${group}`, count: hidden, group });
  };
  for (const entry of groupSessions(others, found).entries) {
    if (entry.kind !== "session") {
      close();
      group = entry.key;
      starting = true;
      shown = 0;
      hidden = 0;
      lines.push({ kind: "label", key: entry.key, text: entry.name, sub: entry.kind === "group" });
      continue;
    }
    const folds = !settledShown.groups.has(group) && !settledShown.keep(entry.info);
    if (entry.child ? parentHidden && folds : shown >= GROUP_SHOWN && folds) {
      if (!entry.child) {
        hidden += 1;
        parentHidden = true;
      }
      continue;
    }
    if (!entry.child) {
      shown += 1;
      parentHidden = false;
    }
    if (starting) groupStarts.push(order.length);
    starting = false;
    add(entry.info, entry.child, false);
  }
  close();

  if (settled.length > 0) {
    lines.push({ kind: "settled", key: "settled", count: settled.length, open: settledShown.open });
    if (settledShown.open) {
      groupStarts.push(order.length);
      const listed = settledShown.all ? settled : settled.slice(0, SETTLED_SHOWN);
      for (const info of listed) add(info, false, true);
      if (listed.length < settled.length) {
        lines.push({ kind: "more", key: "more", count: settled.length - listed.length, group: "settled" });
      }
    }
  }
  return { lines, order, groupStarts };
}

/** The list of a search: the matches in the order given, each by the field it matched. */
export function resultLines(matches: readonly Match[]): Arranged {
  const lines: Line[] = matches.map(({ info, field }) => ({
    kind: "session",
    key: info.id,
    info,
    child: false,
    slim: false,
    field,
  }));
  return {
    lines,
    order: matches.map(({ info }) => info.id),
    groupStarts: matches.slice(0, 1).map(() => 0),
  };
}

/** A project (a repository, or a directory outside one) that has sessions. */
export interface Project {
  repo: string;
  name: string;
  /** Sessions in it, worktrees included. */
  count: number;
  /** The newest activity in it. */
  latest: number;
}

/** The projects the sessions belong to, the most recently active first. */
export function projectsOf(
  sessions: readonly SessionInfo[],
  found: Readonly<Record<string, Place>>,
): Project[] {
  const projects = new Map<string, Project>();
  for (const info of sessions) {
    const repo = placeOf(info.cwd, found).repo;
    const project = projects.get(repo) ?? { repo, name: lastSegment(repo), count: 0, latest: 0 };
    project.count += 1;
    project.latest = Math.max(project.latest, info.lastActivity);
    projects.set(repo, project);
  }
  return [...projects.values()].sort((a, b) => b.latest - a.latest || (a.repo < b.repo ? -1 : 1));
}

/** The sessions of one project; all of them when `repo` is not set. */
export function inProject(
  sessions: readonly SessionInfo[],
  found: Readonly<Record<string, Place>>,
  repo: string | undefined,
): readonly SessionInfo[] {
  if (repo === undefined) return sessions;
  return sessions.filter((info) => placeOf(info.cwd, found).repo === repo);
}

/** The projects whose name or path holds every word of `query`. */
export function matchProjects(projects: readonly Project[], query: string): Project[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return projects.filter((project) => {
    const text = `${project.name} ${project.repo}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
}

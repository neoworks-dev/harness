import { expect, test } from "bun:test";
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import {
  arrange,
  inProject,
  matchProjects,
  projectsOf,
  resultLines,
  SETTLED_SHOWN,
  STALE_MS,
  type Arranged,
  type ShowSettled,
} from "../ui/plugins/sessions/arrange.ts";
import {
  isPinned,
  isSettled,
  mergeMarks,
  NO_MARKS,
  parseMarks,
  pin,
  serializeMarks,
  settle,
  unpin,
  unsettle,
} from "../ui/plugins/sessions/marks.ts";
import { around, highlight, searchSessions, termsOf } from "../ui/plugins/sessions/search.ts";
import {
  age,
  changedLines,
  conventionPlace,
  elapsed,
  finishedAfter,
  groupSessions,
  jumpGroup,
  lastSegment,
  placeOf,
  previewOf,
  runningSinceAfter,
  sessionTitle,
  statusOf,
  step,
  type Entry,
} from "../ui/plugins/sessions/sessions.ts";

function session(id: string, title: string, cwd: string, lastActivity: number): SessionInfo {
  const info: SessionInfo = {
    id,
    cwd,
    lastActivity,
    createdAt: 0,
    lastSeq: 1,
    state: "idle",
    queued: 0,
  };
  if (title) info.title = title;
  return info;
}

/** Two repositories; the first has a main checkout and two worktrees. */
function grouped(extra: SessionInfo[] = []): SessionInfo[] {
  const child = session("d", "Write tests", "/w/site/.worktrees/fix-login", 45);
  child.parent = "c";
  return [
    session("a", "Plan the release", "/w/site", 10),
    session("b", "Docs", "/w/site", 20),
    { ...session("c", "Fix the login", "/w/site/.worktrees/fix-login", 50), state: "running" },
    child,
    session("e", "Dark mode", "/w/site/.worktrees/dark/src", 30),
    session("f", "Scratch", "/w/notes", 40),
    ...extra,
  ];
}

/** Headers as text and sessions as their id (`↳` for a child), top to bottom. */
function outline(entries: readonly Entry[]): string[] {
  return entries.map((e) => {
    if (e.kind === "cluster") return e.name;
    if (e.kind === "group") return `  ⎇ ${e.name}`;
    return `${e.child ? "    ↳ " : "    "}${e.info.id}`;
  });
}

test("sessions group by repository and worktree, newest group first", () => {
  const { entries, order } = groupSessions(grouped(), {});
  expect(outline(entries)).toEqual([
    "site",
    "    b",
    "    a",
    "  ⎇ fix-login",
    "    c",
    "    ↳ d",
    "  ⎇ dark",
    "    e",
    "notes",
    "    f",
  ]);
  expect(order).toEqual(["b", "a", "c", "d", "e", "f"]);
});

test("the cluster with the latest activity leads and worktrees follow their repo", () => {
  const names = (sessions: SessionInfo[]): string[] =>
    outline(groupSessions(sessions, {}).entries).filter((line) => !line.startsWith("    "));
  const withScratch = (last: number): SessionInfo[] =>
    grouped().map((s) => (s.id === "f" ? { ...s, lastActivity: last } : s));
  // The notes directory is quiet; the site repo has the newest session in a worktree.
  expect(names(withScratch(5))).toEqual(["site", "  ⎇ fix-login", "  ⎇ dark", "notes"]);
  // A newer session in the notes directory moves its cluster up.
  expect(names(withScratch(99))).toEqual(["notes", "site", "  ⎇ fix-login", "  ⎇ dark"]);
  // Within a repo, the worktree with the newest activity comes first.
  const darkNewest = withScratch(5).map((s) => (s.id === "e" ? { ...s, lastActivity: 200 } : s));
  expect(names(darkNewest)).toEqual(["site", "  ⎇ dark", "  ⎇ fix-login", "notes"]);
});

test("a child of a session elsewhere is marked but stays in its own group", () => {
  const child = { ...session("k", "Run the build", "/w/site/.worktrees/ci", 20), parent: "p" };
  const { entries } = groupSessions([session("p", "Main chat", "/w/site", 30), child], {});
  expect(outline(entries)).toEqual(["site", "    p", "  ⎇ ci", "    ↳ k"]);
});

test("a worktree the host found on disk groups under its repository", () => {
  const found = { "/w/app-hotfix": { repo: "/w/app", worktree: "hotfix-1" } };
  const { entries } = groupSessions(
    [session("m", "Main", "/w/app", 10), session("h", "Hotfix", "/w/app-hotfix", 20)],
    found,
  );
  expect(outline(entries)).toEqual(["app", "    m", "  ⎇ hotfix-1", "    h"]);
});

test("places come from the worktrees convention, else the directory itself", () => {
  expect(conventionPlace("/nowhere/proj/.worktrees/fix-login/crates/a")).toEqual({
    repo: "/nowhere/proj",
    worktree: "fix-login",
  });
  expect(conventionPlace("/nowhere/proj/.worktrees/")).toBeUndefined();
  expect(placeOf("/nowhere/notes/", {})).toEqual({ repo: "/nowhere/notes" });
});

test("keys skip headers and jump between groups", () => {
  const g = groupSessions(grouped(), {});
  const walk = ["b"];
  for (let i = 0; i < 5; i++) walk.push(step(g.order, walk[walk.length - 1], 1) ?? "?");
  // Main checkout (b, a: newest first), the worktree's session and its child (c, d), then dark
  // (e) and the notes (f): the order on screen.
  expect(walk).toEqual(["b", "a", "c", "d", "e", "f"]);
  expect(step(g.order, "f", 1)).toBe("f");
  expect(step(g.order, "b", -1)).toBe("b");
  expect(step(g.order, undefined, 1)).toBe("a");
  expect(step([], "b", 1)).toBeUndefined();

  expect(jumpGroup(g, "a", 1)).toBe("c");
  expect(jumpGroup(g, "d", 1)).toBe("e");
  expect(jumpGroup(g, "e", 1)).toBe("f");
  expect(jumpGroup(g, "f", 1)).toBe("f");
  expect(jumpGroup(g, "f", -1)).toBe("e");
  expect(jumpGroup(g, "d", -1)).toBe("b");
  expect(jumpGroup(g, "b", -1)).toBe("b");
});

test("titles, previews, ages and changed lines read short", () => {
  expect(sessionTitle(session("cafe0123456", "", "/w", 1))).toBe("session cafe0123");
  expect(sessionTitle(session("a", "  Fix   the\nlogin ", "/w", 1))).toBe("Fix the login");
  expect(previewOf({ ...session("a", "", "/w", 1), preview: "Done.\n I changed  it" })).toBe(
    "Done. I changed it",
  );
  expect(age(0, 59_000)).toBe("now");
  expect(age(0, 5 * 60_000)).toBe("5m");
  expect(age(0, 3 * 3_600_000)).toBe("3h");
  expect(age(0, 2 * 86_400_000)).toBe("2d");
  expect(age(10_000, 0)).toBe("now");
  expect(changedLines({ ...session("a", "", "/w", 1), added: 12, removed: -3 })).toEqual({
    added: 12,
    removed: 0,
  });
  expect(changedLines(session("a", "", "/w", 1))).toEqual({ added: 0, removed: 0 });
});

test("a session that finished unseen is marked until shown", () => {
  const running = (id: string, state: SessionInfo["state"]): SessionInfo => ({
    ...session(id, id, "/w", 10),
    state,
  });
  const before = [running("a", "running"), running("b", "idle")];
  const after = [running("a", "idle"), running("b", "idle")];
  const marked = finishedAfter(new Set(), before, after, "b");
  expect([...marked]).toEqual(["a"]);
  // Nothing changed: the same set comes back, so nothing redraws.
  expect(finishedAfter(marked, after, after, "b")).toBe(marked);
  // Shown once, the mark is gone.
  expect([...finishedAfter(marked, after, after, "a")]).toEqual([]);
  // The shown session finishing doesn't mark it.
  const shownRunning = [running("a", "idle"), running("b", "running")];
  expect([...finishedAfter(new Set(), shownRunning, after, "b")]).toEqual([]);
  // A session that is gone is forgotten.
  expect([...finishedAfter(marked, after, [running("b", "idle")], "b")]).toEqual([]);
  // A session running again loses nothing it had earned, and finishing twice stays one mark.
  expect([...finishedAfter(marked, before, after, "b")]).toEqual(["a"]);
});

/** A session with a last message, for the search. */
function said(id: string, title: string, preview: string, cwd = "/w/site", at = 10): SessionInfo {
  return { ...session(id, title, cwd, at), preview };
}

const projectName = (info: SessionInfo): string => lastSegment(placeOf(info.cwd, {}).repo);

test("a search ranks the title, then the last message, then the project", () => {
  const all = [
    said("p", "Docs", "I fixed the login page", "/w/site", 50),
    said("t", "Fix the login", "Done", "/w/other", 10),
    said("r", "Release", "nothing", "/w/login-kit", 40),
    said("x", "Login redirect", "Done", "/w/other", 20),
  ];
  const ids = (query: string): string[] =>
    searchSessions(all, query, projectName).map((m) => m.info.id);
  // A title that starts with the word comes before one that holds it, then the message, then the
  // project.
  expect(ids("login")).toEqual(["x", "t", "p", "r"]);
  expect(searchSessions(all, "login", projectName).map((m) => m.field)).toEqual([
    "title",
    "title",
    "preview",
    "project",
  ]);
  // Every word has to be found, in any order and case.
  expect(ids("LOGIN fix")).toEqual(["t", "p"]);
  // Words spread over the title and the project still match.
  expect(ids("release kit")).toEqual(["r"]);
  expect(ids("zzz")).toEqual([]);
  expect(ids("   ")).toEqual([]);
});

test("equal matches come newest first", () => {
  const all = [said("a", "Fix", "", "/w", 10), said("b", "Fix", "", "/w", 30)];
  expect(searchSessions(all, "fix", projectName).map((m) => m.info.id)).toEqual(["b", "a"]);
});

test("a match is cut into runs and a snippet that shows it", () => {
  expect(termsOf(" Fix  fix LOGIN ")).toEqual(["fix", "login"]);
  expect(highlight("Fix the login", ["fix", "login"])).toEqual([
    { text: "Fix", hit: true },
    { text: " the ", hit: false },
    { text: "login", hit: true },
  ]);
  expect(highlight("aaa", ["aa"])).toEqual([{ text: "aaa", hit: true }]);
  expect(highlight("nothing", ["zz"])).toEqual([{ text: "nothing", hit: false }]);
  expect(highlight("", ["a"])).toEqual([{ text: "", hit: false }]);
  const long = "I looked through the whole repository and fixed the login page";
  expect(around(long, ["login"])).toBe("…d fixed the login page");
  expect(around("short login", ["login"])).toBe("short login");
  expect(around(long, ["zz"])).toBe(long);
});

test("a settled session is put away until something happens in it", () => {
  const quiet = session("a", "Quiet", "/w", 100);
  let marks = settle(NO_MARKS, quiet);
  expect(isSettled(marks, quiet)).toBe(true);
  // Any later activity brings it back, without un-settling it first.
  expect(isSettled(marks, { ...quiet, lastActivity: 101 })).toBe(false);
  marks = unsettle(marks, "a");
  expect(isSettled(marks, quiet)).toBe(false);
  expect(isSettled(NO_MARKS, quiet)).toBe(false);
});

test("a session is pinned or settled, not both", () => {
  const a = session("a", "A", "/w", 5);
  const pinned = pin(NO_MARKS, "a", 7);
  expect(isPinned(pinned, "a")).toBe(true);
  const settled = settle(pinned, a);
  expect(isPinned(settled, "a")).toBe(false);
  expect(isSettled(settled, a)).toBe(true);
  const again = pin(settled, "a", 9);
  expect(isSettled(again, a)).toBe(false);
  expect(again.pinned).toEqual({ a: 9 });
  expect(isPinned(unpin(again, "a"), "a")).toBe(false);
  // Merging keeps the later say about a session.
  const merged = mergeMarks(settle(NO_MARKS, a), pin(NO_MARKS, "a", 1));
  expect(isPinned(merged, "a")).toBe(true);
  expect(isSettled(merged, a)).toBe(false);
});

test("marks are read back from the file they were written to", () => {
  const marks = pin(settle(NO_MARKS, session("s", "S", "/w", 3)), "p", 4);
  expect(parseMarks(serializeMarks(marks))).toEqual(marks);
  expect(parseMarks("")).toEqual(NO_MARKS);
  expect(parseMarks("[1]")).toEqual(NO_MARKS);
  expect(parseMarks('{"pinned":{"a":1,"b":"x"},"settled":5}')).toEqual({
    pinned: { a: 1 },
    settled: {},
  });
});

/** Ids as the list draws them: labels in brackets, `↳` for a child, `~` for a slim row. */
function drawn(arranged: Arranged): string[] {
  return arranged.lines.map((line) => {
    if (line.kind === "label") return `[${line.text}]`;
    if (line.kind === "settled") return `[Settled ${line.count} ${line.open}]`;
    if (line.kind === "more") return `[more ${line.count}]`;
    let mark = "";
    if (line.child) mark += "↳";
    if (line.slim) mark += "~";
    return mark + line.info.id;
  });
}

/** How the list is shown: nothing opened, nothing kept from folding, nothing old yet, unless said otherwise. */
function show(open: boolean, all: boolean, more: Partial<ShowSettled> = {}): ShowSettled {
  return { open, all, groups: new Set(), keep: () => false, nowMs: 0, ...more };
}

test("the list puts pinned first and settled last, closed until opened", () => {
  const all = grouped();
  const scratch = session("f", "Scratch", "/w/notes", 40);
  const marks = settle(pin(pin(NO_MARKS, "a", 1), "e", 2), scratch);
  const closed = arrange(all, {}, marks, show(false, false));
  // Pinned last first, the rest under their repository and worktree (b, then c with its child
  // d), settled at the end.
  expect(drawn(closed)).toEqual([
    "[Pinned]",
    "e",
    "a",
    "[site]",
    "b",
    "[fix-login]",
    "c",
    "↳d",
    "[Settled 1 false]",
  ]);
  expect(closed.order).toEqual(["e", "a", "b", "c", "d"]);
  expect(closed.groupStarts).toEqual([0, 2, 3]);
  const open = arrange(all, {}, marks, show(true, false));
  expect(drawn(open).slice(-2)).toEqual(["[Settled 1 true]", "~f"]);
  expect(open.order.at(-1)).toBe("f");
  // New activity in a settled session puts it back among the rest.
  const woken = all.map((s) => (s.id === "f" ? { ...s, lastActivity: 99 } : s));
  expect(drawn(arrange(woken, {}, marks, show(false, false)))).toEqual([
    "[Pinned]",
    "e",
    "a",
    "[notes]",
    "f",
    "[site]",
    "b",
    "[fix-login]",
    "c",
    "↳d",
  ]);
  // Nothing marked is the grouped list as it was.
  expect(arrange(all, {}, NO_MARKS, show(false, false)).order).toEqual(groupSessions(all, {}).order);
});

test("as in the terminal, sessions idle for over a day fold into their group's line until it is opened", () => {
  const now = 10 * STALE_MS;
  const fresh = (id: string, ago: number): SessionInfo => session(id, id, "/w/site", now - ago);
  const all = [
    fresh("new", 1000),
    fresh("hour", 3600_000),
    fresh("old", STALE_MS + 1),
    fresh("older", 3 * STALE_MS),
    // An old one that works stays in sight, and so does a child of a shown session, though old.
    { ...fresh("busy", 5 * STALE_MS), state: "running" as const },
    { ...fresh("kid", 4 * STALE_MS), parent: "new" },
  ];
  const keep = (s: SessionInfo): boolean => s.state === "running";
  const folded = arrange(all, {}, NO_MARKS, show(false, false, { keep, nowMs: now }));
  expect(drawn(folded)).toEqual(["[site]", "new", "↳kid", "hour", "busy", "[more 2]"]);
  expect(folded.lines.at(-1)).toMatchObject({ group: "repo /w/site" });
  const opened = arrange(all, {}, NO_MARKS, show(false, false, { keep, nowMs: now, groups: new Set(["repo /w/site"]) }));
  expect(opened.order).toHaveLength(6);
});

test("settled sessions show ten, then the rest on request", () => {
  const many = Array.from({ length: 14 }, (_, i) => session(`s${i}`, `S${i}`, "/w", 100 + i));
  const marks = many.reduce((m, info) => settle(m, info), NO_MARKS);
  const some = drawn(arrange(many, {}, marks, show(true, false)));
  expect(some).toHaveLength(1 + SETTLED_SHOWN + 1);
  expect(some[1]).toBe("~s13");
  expect(some.at(-1)).toBe("[more 4]");
  const every = drawn(arrange(many, {}, marks, show(true, true)));
  expect(every).toHaveLength(1 + 14);
  expect(every.at(-1)).toBe("~s0");
});

test("projects list by latest activity and filter the sessions", () => {
  const all = grouped();
  const projects = projectsOf(all, {});
  expect(projects.map((p) => [p.name, p.count])).toEqual([
    ["site", 5],
    ["notes", 1],
  ]);
  expect(projects[0]?.repo).toBe("/w/site");
  expect(inProject(all, {}, "/w/notes").map((s) => s.id)).toEqual(["f"]);
  expect(inProject(all, {}, undefined)).toBe(all);
  expect(inProject(all, {}, "/w/gone")).toEqual([]);
  expect(matchProjects(projects, "NOT").map((p) => p.name)).toEqual(["notes"]);
  expect(matchProjects(projects, "w si").map((p) => p.name)).toEqual(["site"]);
  expect(matchProjects(projects, "")).toHaveLength(2);
});

test("a search lists its matches as they come", () => {
  const all = [said("a", "Fix", "", "/w", 10), said("b", "Fixed", "", "/w", 30)];
  const results = resultLines(searchSessions(all, "fix", projectName));
  expect(results.order).toEqual(["b", "a"]);
  expect(drawn(results)).toEqual(["b", "a"]);
  expect(resultLines([]).groupStarts).toEqual([]);
});

test("a row says working, done or nothing, and for how long", () => {
  const running = { ...session("a", "A", "/w", 1), state: "running" as const };
  expect(statusOf(running, false, { a: 500 })).toEqual({ kind: "working", since: 500 });
  expect(statusOf(running, true, {})).toEqual({ kind: "working", since: undefined });
  expect(statusOf(session("a", "A", "/w", 1), true, {})).toEqual({ kind: "done" });
  expect(statusOf(session("a", "A", "/w", 1), false, {})).toEqual({ kind: "idle" });
  expect(elapsed(0, 12_000)).toBe("12s");
  expect(elapsed(0, 192_000)).toBe("3m 12s");
  expect(elapsed(0, 3_725_000)).toBe("1h 02m");
  expect(elapsed(5000, 0)).toBe("0s");

  const idle = session("b", "B", "/w", 1);
  const since = runningSinceAfter({}, [running, idle], 900);
  expect(since).toEqual({ a: 900 });
  // Kept while it runs, and the same object comes back when nothing changed.
  expect(runningSinceAfter(since, [running, idle], 5000)).toBe(since);
  expect(runningSinceAfter(since, [idle], 6000)).toEqual({});
});

import { expect, test } from "bun:test";
import type { SessionInfo } from "@suite/protocol/types";
import {
  age,
  changedLines,
  conventionPlace,
  finishedAfter,
  groupSessions,
  jumpGroup,
  placeOf,
  previewOf,
  sessionTitle,
  step,
  type Entry,
} from "../ui/lib/sessions.ts";

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

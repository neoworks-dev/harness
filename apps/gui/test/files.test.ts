import { expect, test } from "bun:test";
import type { SessionEvent, ToolCallContent } from "@suite/protocol/types";
import {
  NO_TOUCHES,
  applyTouch,
  branchLabel,
  fileEntries,
  glyphOfMark,
  parseStatus,
  relFromPrefix,
  relativize,
  type Touches,
} from "../ui/plugins/files/files.ts";

const NUL = "\0";

test("porcelain v2 status is read: branch, changes, renames, conflicts, untracked", () => {
  const out = [
    "# branch.oid abc",
    "# branch.head main",
    "# branch.ab +2 -1",
    "1 .M N... 100644 100644 100644 aaa bbb src/a b.ts",
    "1 A. N... 000000 100644 100644 000 ccc new.ts",
    "1 .D N... 100644 100644 000000 aaa aaa gone.ts",
    `2 R. N... 100644 100644 100644 aaa bbb R100 moved.ts${NUL}old.ts`,
    "u UU N... 100644 100644 100644 100644 a b c both.ts",
    "? scratch.txt",
    "",
  ].join(NUL);
  const status = parseStatus(out, "");
  expect(branchLabel(status)).toBe("main ↑2 ↓1");
  expect(status.entries).toEqual([
    { path: "src/a b.ts", change: "modified" },
    { path: "new.ts", change: "added" },
    { path: "gone.ts", change: "deleted" },
    { path: "moved.ts", orig: "old.ts", change: "renamed" },
    { path: "both.ts", change: "conflicted" },
    { path: "scratch.txt", change: "untracked" },
  ]);
  expect(branchLabel(parseStatus(["# branch.head (detached)", ""].join(NUL)))).toBe("detached");
  expect(parseStatus("").entries).toEqual([]);
});

test("paths are shown from the session's directory", () => {
  expect(relFromPrefix("", "a/b.ts")).toBe("a/b.ts");
  expect(relFromPrefix("src/", "src/a.ts")).toBe("a.ts");
  expect(relFromPrefix("src/a/", "src/b.ts")).toBe("../b.ts");
  expect(relFromPrefix("src/", "docs/x/")).toBe("../docs/x/");
  expect(relativize("/w/site", "/w/site/src/a.ts")).toBe("src/a.ts");
  expect(relativize("/w/site/", "/elsewhere/a.ts")).toBe("/elsewhere/a.ts");
  expect(relativize("/w/site", "./a.ts")).toBe("a.ts");
  expect(relativize("/w/site", "/w/site")).toBe(".");
});

let seq = 0;
function touches(state: Touches, event: SessionEvent): Touches {
  return applyTouch(state, { session: "s", seq: ++seq, ts: 0, event }, "/w/site");
}

function edit(id: string, path: string, oldText?: string): SessionEvent {
  const diff: ToolCallContent = { type: "diff", path, newText: "x" };
  if (oldText !== undefined) diff.oldText = oldText;
  return { type: "tool_call_update", toolCallId: id, content: [diff] };
}

test("files agents touched are listed, tagged by who, with or without git", () => {
  let state = NO_TOUCHES;
  state = touches(state, {
    type: "tool_call",
    toolCallId: "t1",
    title: "edit",
    kind: "edit",
    status: "pending",
  });
  state = touches(state, edit("t1", "/w/site/src/a.ts", "old"));
  state = touches(state, {
    type: "tool_call",
    toolCallId: "t2",
    title: "edit",
    kind: "edit",
    status: "pending",
    agent: "claude",
  });
  state = touches(state, edit("t2", "/w/site/src/a.ts", "old"));
  state = touches(state, edit("t2", "/w/site/gone.txt"));
  // Not a diff, or no path: nothing.
  expect(touches(state, { type: "tool_call_update", toolCallId: "t1", status: "completed" })).toBe(
    state,
  );
  expect(touches(state, edit("t1", ""))).toBe(state);

  const offline = fileEntries(undefined, state.touched);
  expect(offline).toEqual([
    { rel: "src/a.ts", mark: "modified", tags: ["agent", "claude"] },
    { rel: "gone.txt", mark: "added", tags: ["claude"] },
  ]);

  const status = parseStatus(
    ["# branch.head main", "1 .M N... 100644 100644 100644 a b src/a.ts", "? b.txt", ""].join(NUL),
  );
  const entries = fileEntries(status, state.touched);
  expect(entries.map((e) => [e.rel, e.mark, e.tags.join("+")])).toEqual([
    ["src/a.ts", "modified", "agent+claude"],
    ["b.txt", "untracked", ""],
    // Touched, but git sees no difference (committed since).
    ["gone.txt", "clean", "claude"],
  ]);
  expect(entries.map((e) => glyphOfMark(e.mark)).join("")).toBe("M?·");
});

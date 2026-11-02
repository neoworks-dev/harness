import { describe, expect, test } from "bun:test";
import type { Component, UiNode } from "@neoworks-dev/neo-protocol/types";
import { recognize } from "../src/view/recognize.ts";

/** The neoui root a recognized output draws (the first one, in a stack). */
function root(c: Component | undefined): UiNode | undefined {
  if (c?.type === "neoui") return c.ui.root;
  if (c?.type === "stack") return c.children.map(root).find((r) => r !== undefined);
  return undefined;
}

const props = (n: UiNode | undefined): Record<string, unknown> => n?.props ?? {};
const kids = (n: UiNode | undefined, prop: string): UiNode[] => (props(n)[prop] as UiNode[]) ?? [];

describe("test runs", () => {
  test("cargo test: every binary summed, each failure with its panic", () => {
    const out = `running 3 tests
test tests::later ... ignored
test tests::adds ... ok
test tests::subtracts ... FAILED

failures:

---- tests::subtracts stdout ----

thread 'tests::subtracts' (34470) panicked at src/lib.rs:6:22:
assertion \`left == right\` failed: subtraction is off
  left: 1
 right: 3
note: run with \`RUST_BACKTRACE=1\` environment variable to display a backtrace


failures:
    tests::subtracts

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.50s

running 2 tests
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.25s
`;
    const c = recognize("cd crate && cargo test 2>&1 | tail -40", out);
    const run = root(c);
    expect(run?.component).toBe("TestRun");
    expect(props(run)).toMatchObject({ passed: 3, failed: 1, skipped: 1, duration: "1.75s" });
    const [failed] = kids(run, "tests");
    expect(props(failed)).toEqual({
      name: "tests::subtracts",
      status: "failed",
      message:
        "thread 'tests::subtracts' (34470) panicked at src/lib.rs:6:22:\nassertion `left == right` failed: subtraction is off\n  left: 1\n right: 3",
    });
    // The output itself stays under the summary.
    expect(c?.type === "stack" && c.children[1]?.type).toBe("code");
  });

  test("bun test: the counts and each (fail) with its error", () => {
    const out = `a.test.ts:
3 | test("subtracts wrongly", () => { expect(2 - 1).toBe(3); });
                                                    ^
error: expect(received).toBe(expected)

Expected: 3
Received: 1

      at <anonymous> (/tmp/a.test.ts:3:49)
(fail) subtracts wrongly [1.00ms]

 1 pass
 1 skip
 1 fail
Ran 3 tests across 1 file. [1.50s]`;
    const run = root(recognize("bun test", out));
    expect(props(run)).toMatchObject({ passed: 1, failed: 1, skipped: 1, duration: "1.5s" });
    expect(props(kids(run, "tests")[0])).toMatchObject({
      name: "subtracts wrongly",
      message:
        "error: expect(received).toBe(expected)\nExpected: 3\nReceived: 1\n      at <anonymous> (/tmp/a.test.ts:3:49)",
    });
  });

  test("go test: failures with or without -v; all ok without -v is left alone", () => {
    const plain =
      "--- FAIL: TestSubtracts (0.00s)\n    a_test.go:4: got 1, want 3\nFAIL\nFAIL\trt\t0.20s\n";
    expect(props(root(recognize("go test ./...", plain)))).toMatchObject({
      failed: 1,
      duration: "0.2s",
    });
    const verbose =
      "=== RUN   TestAdds\n--- PASS: TestAdds (0.00s)\n=== RUN   TestSubtracts\n    a_test.go:4: got 1, want 3\n--- FAIL: TestSubtracts (0.00s)\nFAIL\nFAIL\trt\t0.001s\n";
    const run = root(recognize("go test -v ./...", verbose));
    expect(props(run)).toMatchObject({ passed: 1, failed: 1 });
    expect(props(run).duration).toBeUndefined();
    expect(props(kids(run, "tests")[0]).message).toBe("a_test.go:4: got 1, want 3");
    expect(recognize("go test ./...", "ok  \trt\t0.01s\n")).toBeUndefined();
  });

  test("pytest, jest and vitest summaries", () => {
    const py = `FAILED tests/test_math.py::test_sub - assert 1 == 3
=================== 1 failed, 10 passed, 2 skipped in 1.23s ===================`;
    const run = root(recognize("python -m pytest -q", py));
    expect(props(run)).toMatchObject({ passed: 10, failed: 1, skipped: 2, duration: "1.23s" });
    expect(props(kids(run, "tests")[0])).toMatchObject({
      name: "tests/test_math.py::test_sub",
      message: "assert 1 == 3",
    });

    const jest = `  ● math › subtracts

    expect(received).toBe(expected)

Tests:       1 failed, 10 passed, 11 total
Time:        2.5 s`;
    expect(props(root(recognize("npx jest", jest)))).toMatchObject({
      passed: 10,
      failed: 1,
      duration: "2.5s",
    });

    const vitest = ` FAIL  test/math.test.ts > math > subtracts
 Tests  1 failed | 10 passed (11)
 Duration  800ms`;
    const v = root(recognize("pnpm test", vitest));
    expect(props(v)).toMatchObject({ passed: 10, failed: 1, duration: "0.8s" });
    expect(props(kids(v, "tests")[0]).name).toBe("test/math.test.ts > math > subtracts");
  });
});

describe("git", () => {
  test("git diff: a diff per file, lines numbered", () => {
    const out = `diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,2 +10,2 @@ fn x
 ctx
-old
+new
diff --git a/b.md b/b.md
--- a/b.md
+++ b/b.md
@@ -1 +1 @@
-a
+b
`;
    const c = recognize("git diff HEAD~1", out);
    if (c?.type !== "stack") throw new Error(`a stack of diffs, got ${c?.type}`);
    const diffs = c.children.filter((d) => d.type === "diff");
    expect(diffs.map((d) => d.path)).toEqual(["src/a.rs", "b.md"]);
    expect(diffs[0]?.lines.slice(1)).toEqual([
      { kind: "context", text: "ctx", old: 10, new: 10 },
      { kind: "remove", text: "old", old: 11 },
      { kind: "add", text: "new", new: 11 },
    ]);
  });

  test("git status, short and long", () => {
    const short = " M Cargo.lock\nA  src/new.rs\nR  old.rs -> moved.rs\n?? notes/\n";
    const files = kids(root(recognize("git status --short", short)), "files").map(props);
    expect(files).toEqual([
      { path: "Cargo.lock", status: "modified" },
      { path: "src/new.rs", status: "added" },
      { path: "moved.rs", status: "renamed" },
      { path: "notes/", status: "untracked" },
    ]);
    const long = `On branch main
Changes not staged for commit:
\tmodified:   Cargo.lock
\tdeleted:    gone.txt

Untracked files:
  (use "git add <file>..." to include in what will be committed)
\tnotes/
`;
    expect(kids(root(recognize("git status", long)), "files").map(props)).toEqual([
      { path: "Cargo.lock", status: "modified" },
      { path: "gone.txt", status: "deleted" },
      { path: "notes/", status: "untracked" },
    ]);
  });

  test("git log, one line or in full", () => {
    const oneline = "27b08b9 (HEAD -> main) neoui Dev components\nfece03c term: an expanded card\n";
    const commits = kids(root(recognize("git log --oneline -2", oneline)), "children").map(props);
    expect(commits).toEqual([
      { sha: "27b08b9", title: "neoui Dev components" },
      { sha: "fece03c", title: "term: an expanded card" },
    ]);
    const full = `commit 27b08b9f54401ee46fd897a2106700afdeaf79b4
Author: Moritz Utcke <m@example.com>
Date:   Mon Nov 2 16:38:55 2026 +0100

    neoui Dev components in term

    The body.
`;
    expect(props(root(recognize("git log -1", full)))).toEqual({
      sha: "27b08b9f54401ee46fd897a2106700afdeaf79b4",
      title: "neoui Dev components in term",
      author: "Moritz Utcke",
      date: "Mon Nov 2 16:38:55 2026 +0100",
    });
  });
});

describe("gh", () => {
  test("gh pr view without a terminal: the card, then the body", () => {
    const out = `title:\tBump goldmark
state:\tOPEN
author:\tdependabot
labels:\tdependencies, go
number:\t14907
url:\thttps://github.com/cli/cli/pull/14907
additions:\t3
deletions:\t3
--
Bumps goldmark.`;
    const c = recognize("gh pr view 14907 -R cli/cli", out);
    expect(props(root(c))).toEqual({
      number: 14907,
      title: "Bump goldmark",
      state: "open",
      author: "dependabot",
      url: "https://github.com/cli/cli/pull/14907",
      repo: "cli/cli",
      added: 3,
      removed: 3,
    });
    expect(c?.type === "stack" && c.children[1]?.type).toBe("text");
  });

  test("gh issue view and the lists", () => {
    const issue =
      "title:\tA bug\nstate:\tOPEN\nauthor:\tbot\nlabels:\tarea:ui, bug\ncomments:\t2\nnumber:\t53\n--\n";
    expect(props(root(recognize("gh issue view 53", issue)))).toEqual({
      number: 53,
      title: "A bug",
      state: "open",
      author: "bot",
      labels: ["area:ui", "bug"],
      comments: 2,
    });
    const prs = "14907\tBump goldmark\tdependabot/goldmark\tOPEN\t2026-11-02T14:03:42Z\n";
    expect(kids(root(recognize("gh pr list", prs)), "children").map(props)).toEqual([
      { number: 14907, title: "Bump goldmark", branch: "dependabot/goldmark", state: "open" },
    ]);
    const issues = "55\tOPEN\tGet models to use neoui\tarea:ui\t2026-11-02T15:39:41Z\n";
    expect(kids(root(recognize("gh issue list", issues)), "children").map(props)).toEqual([
      { number: 55, title: "Get models to use neoui", state: "open", labels: ["area:ui"] },
    ]);
  });

  test("gh --json: the review and the checks summed up", () => {
    const json = JSON.stringify({
      number: 41,
      title: "Defer prompts",
      state: "OPEN",
      isDraft: true,
      author: { login: "moritz" },
      headRefName: "defer",
      baseRefName: "main",
      reviewDecision: "CHANGES_REQUESTED",
      statusCheckRollup: [{ conclusion: "SUCCESS" }, { conclusion: "FAILURE" }],
    });
    expect(props(root(recognize("gh pr view 41 --json number,title", json)))).toEqual({
      number: 41,
      title: "Defer prompts",
      state: "draft",
      author: "moritz",
      branch: "defer",
      base: "main",
      review: "changes",
      checks: "failing",
    });
  });

  test("gh pr checks", () => {
    const out =
      "build\tpass\t2m10s\thttps://x/1\t\ntest\tfail\t4m2s\thttps://x/2\t\nlint\tpending\t0\t\t\n";
    expect(kids(root(recognize("gh pr checks 41", out)), "checks").map(props)).toEqual([
      { name: "build", status: "passed", duration: "2m10s", url: "https://x/1" },
      { name: "test", status: "failed", duration: "4m2s", url: "https://x/2" },
      { name: "lint", status: "pending" },
    ]);
  });
});

test("what isn't recognized stays plain", () => {
  expect(recognize("cargo build", "   Compiling neo v0.1.0\n")).toBeUndefined();
  expect(recognize("cargo test", "error[E0308]: mismatched types\n")).toBeUndefined();
  expect(recognize("git diff", "")).toBeUndefined();
  expect(
    recognize("git status", "On branch main\nnothing to commit, working tree clean\n"),
  ).toBeUndefined();
  expect(recognize("gh pr view 1", "no pull requests found\n")).toBeUndefined();
  expect(recognize("ls", "a\nb\n")).toBeUndefined();
});

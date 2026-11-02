// A shell command's output read as what it is: a test run as a TestRun, `git diff` as diffs,
// `git status` as a file tree, `git log` as commits, `gh` as pull requests, issues and checks
// (docs/neoui.md, the Dev components). The chat then shows a command's result the way a developer
// reads it without the model doing anything. What isn't recognized stays plain output.
import type { Component, DiffLine, UiNode } from "@neoworks-dev/neo-protocol/types";
import { parseMarkdown } from "./markdown.ts";

const node = (component: string, props: Record<string, unknown>): UiNode => ({ component, props });

/** A neoui doc of one node, as a tool result's component. */
function doc(root: UiNode): Component {
  return { type: "neoui", ui: { id: "0", root, streaming: false } };
}

/** A terminal color code (`ESC[1;31m`): output can carry them when a tool forces colors. */
const ANSI_COLOR = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, "g");

/** What `command` printed (`output`), as components; undefined when it is nothing we know. */
export function recognize(command: string, output: string): Component | undefined {
  const text = output.replace(/\r/g, "").replace(ANSI_COLOR, "");
  if (text.trim() === "") return undefined;
  for (const reader of READERS) {
    if (!reader.when.test(command)) continue;
    const got = reader.read(text, command);
    if (got !== undefined) return got;
  }
  return undefined;
}

interface Reader {
  /** The command runs this program (anywhere in a pipeline or after a `cd … &&`). */
  when: RegExp;
  read(output: string, command: string): Component | undefined;
}

// ─── Test runs ──────────────────────────────────────────────────────────────

interface Run {
  passed?: number;
  failed?: number;
  skipped?: number;
  duration?: string;
  tests: { name: string; message?: string }[];
}

/** A test run's summary over the full output (the counts say little without the failures). */
function testRun(name: string, run: Run, output: string): Component {
  const props: Record<string, unknown> = { name };
  if (run.passed !== undefined) props.passed = run.passed;
  if (run.failed !== undefined) props.failed = run.failed;
  if (run.skipped !== undefined && run.skipped > 0) props.skipped = run.skipped;
  if (run.duration !== undefined) props.duration = run.duration;
  props.tests = run.tests.slice(0, 20).map((t) =>
    node("Test", {
      name: t.name,
      status: "failed",
      message: t.message,
    }),
  );
  // A test without a message has none: the prop is left out, not empty.
  for (const t of props.tests as UiNode[]) {
    if (t.props.message === undefined || t.props.message === "") delete t.props.message;
  }
  return {
    type: "stack",
    gap: "s",
    children: [doc(node("TestRun", props)), { type: "code", code: output.replace(/\n+$/, "") }],
  };
}

/** The first lines of a failure's text, blank lines and runner noise left out. */
function message(lines: string[]): string {
  return lines
    .filter((l) => l.trim() !== "" && !/^note: run with `RUST_BACKTRACE/.test(l))
    .slice(0, 12)
    .join("\n");
}

function seconds(total: number): string {
  return total >= 60
    ? `${Math.floor(total / 60)}m ${Math.round(total % 60)}s`
    : `${+total.toFixed(2)}s`;
}

/** A run's time as its `duration`; none below a hundredth of a second, which says nothing. */
function took(total: number): { duration?: string } {
  const out: { duration?: string } = {};
  if (total >= 0.01) out.duration = seconds(total);
  return out;
}

/** `cargo test`: every binary's `test result:` line summed, and each `---- name stdout ----`. */
function cargoTest(output: string): Run | undefined {
  const results = [
    ...output.matchAll(
      /^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;.*?(?:finished in ([\d.]+)s)?$/gm,
    ),
  ];
  if (results.length === 0) return undefined;
  const sum = (i: number): number => results.reduce((n, m) => n + Number(m[i] ?? 0), 0);
  const time = results.reduce((n, m) => n + Number(m[4] ?? 0), 0);
  const tests: Run["tests"] = [];
  const lines = output.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const head = /^---- (.+?) stdout ----$/.exec(lines[i] ?? "");
    if (!head?.[1]) continue;
    const body: string[] = [];
    for (i++; i < lines.length && !/^(---- .+ ----|failures:)$/.test(lines[i] ?? ""); i++) {
      body.push(lines[i] ?? "");
    }
    i--;
    tests.push({ name: head[1], message: message(body) });
  }
  // Failed without its output captured (`--nocapture`): the names under the last `failures:`.
  if (tests.length === 0 && sum(2) > 0) {
    for (const m of output.matchAll(/^test (\S+) \.\.\. FAILED$/gm))
      tests.push({ name: m[1] ?? "" });
  }
  return { passed: sum(1), failed: sum(2), skipped: sum(3), ...took(time), tests };
}

/** `bun test`: ` 515 pass`, ` 1 fail`, and each `(fail) name` with the error printed before it. */
function bunTest(output: string): Run | undefined {
  const pass = /^\s*(\d+) pass$/m.exec(output);
  const fail = /^\s*(\d+) fail$/m.exec(output);
  if (!pass && !fail) return undefined;
  const skip = /^\s*(\d+) skip$/m.exec(output);
  const time = /^Ran \d+ tests? across .*\[([\d.]+)(m?s)\]$/m.exec(output);
  const lines = output.split("\n");
  const tests: Run["tests"] = [];
  let since = 0;
  lines.forEach((line, i) => {
    const failed = /^\(fail\) (.+?)(?: \[[\d.]+m?s\])?$/.exec(line);
    if (/^\((pass|fail|skip|todo)\)/.test(line) || /^\S.*\.(test|spec)\.\w+:$/.test(line)) {
      if (failed?.[1]) {
        const before = lines.slice(since, i);
        // From the error itself; the source excerpt (`3 | test(…`) printed above it is noise here.
        let from = before.findIndex((l) => /^error/.test(l));
        if (from < 0) from = before.findIndex((l) => /^(Expected|Received|\s*at )/.test(l));
        const test: Run["tests"][number] = { name: failed[1] };
        if (from >= 0) test.message = message(before.slice(from));
        tests.push(test);
      }
      since = i + 1;
    }
  });
  const total = time?.[1] === undefined ? 0 : Number(time[1]) / (time[2] === "ms" ? 1000 : 1);
  return {
    passed: Number(pass?.[1] ?? 0),
    failed: Number(fail?.[1] ?? 0),
    skipped: Number(skip?.[1] ?? 0),
    ...took(total),
    tests,
  };
}

/**
 * `go test`: `--- PASS/FAIL/SKIP` lines (with -v), else failures only. A failure's text is
 * indented under its `--- FAIL` line, or with -v logged between its `=== RUN` and that line.
 */
function goTest(output: string): Run | undefined {
  const count = (word: string): number =>
    [...output.matchAll(new RegExp(`^\\s*--- ${word}: `, "gm"))].length;
  const packages = [...output.matchAll(/^(ok|FAIL)\s+\S+\s+([\d.]+)s/gm)];
  if (packages.length === 0 && count("FAIL") === 0) return undefined;
  const lines = output.split("\n");
  const tests: Run["tests"] = [];
  lines.forEach((line, i) => {
    const m = /^\s*--- FAIL: (\S+)/.exec(line);
    if (!m?.[1]) return;
    const indent = /^\s*/.exec(line)?.[0].length ?? 0;
    const body: string[] = [];
    for (let j = i + 1; j < lines.length; j++) {
      const next = lines[j] ?? "";
      if ((/^\s*/.exec(next)?.[0].length ?? 0) <= indent) break;
      body.push(next.trim());
    }
    if (body.length === 0) {
      const run = lines.lastIndexOf(`=== RUN   ${m[1]}`, i);
      if (run >= 0) body.push(...lines.slice(run + 1, i).map((l) => l.trim()));
    }
    tests.push({ name: m[1], message: message(body) });
  });
  const time = packages.reduce((n, m) => n + Number(m[2] ?? 0), 0);
  const run: Run = { failed: count("FAIL"), ...took(time), tests };
  if (count("PASS") > 0) run.passed = count("PASS");
  if (count("SKIP") > 0) run.skipped = count("SKIP");
  // Without -v all that says it passed is `ok` per package: nothing to count, say so in words.
  if (run.passed === undefined && run.failed === 0) return undefined;
  return run;
}

/** pytest: `== 1 failed, 10 passed, 2 skipped in 1.23s ==` and its `FAILED path::name - why` lines. */
function pytest(output: string): Run | undefined {
  const summary = /^=+ (.*\b(?:passed|failed|error)\b.*) in ([\d.]+)s/m.exec(output);
  if (!summary?.[1]) return undefined;
  const n = (word: string): number =>
    Number(new RegExp(`(\\d+) ${word}`).exec(summary[1] ?? "")?.[1] ?? 0);
  const tests = [...output.matchAll(/^FAILED (\S+)(?: - (.*))?$/gm)].map((m) => {
    const test: Run["tests"][number] = { name: m[1] ?? "" };
    if (m[2] !== undefined) test.message = m[2];
    return test;
  });
  return {
    passed: n("passed"),
    failed: n("failed") + n("errors?"),
    skipped: n("skipped"),
    ...took(Number(summary[2])),
    tests,
  };
}

/** jest (`Tests: 1 failed, 10 passed, 11 total`, `● a › b`) and vitest (`Tests  1 failed | 10 passed`, `FAIL file > a > b`). */
function jsTest(output: string): Run | undefined {
  const line = /^\s*Tests:?\s+(.*(?:passed|failed).*)$/m.exec(output)?.[1];
  if (line === undefined) return undefined;
  const n = (word: string): number => Number(new RegExp(`(\\d+) ${word}`).exec(line)?.[1] ?? 0);
  const names = new Set<string>();
  for (const m of output.matchAll(/^\s*● (.+ › .+)$/gm)) names.add(m[1]?.trim() ?? "");
  for (const m of output.matchAll(/^\s*(?:FAIL|×|✗)\s+(\S+ > .+?)(?: \d+ms)?$/gm)) {
    names.add(m[1]?.trim() ?? "");
  }
  const time = /^\s*(?:Time|Duration):?\s+([\d.]+)\s*(m?s)/m.exec(output);
  const run: Run = {
    passed: n("passed"),
    failed: n("failed"),
    skipped: n("skipped"),
    tests: [...names].filter((x) => x !== "").map((name) => ({ name })),
  };
  if (time?.[1] !== undefined)
    run.duration = seconds(Number(time[1]) / (time[2] === "ms" ? 1000 : 1));
  return run;
}

// ─── git ────────────────────────────────────────────────────────────────────

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** A unified diff as one diff per file, each hunk's lines numbered. */
export function parseUnified(text: string): { path: string; lines: DiffLine[] }[] {
  const files: { path: string; lines: DiffLine[] }[] = [];
  let file: { path: string; lines: DiffLine[] } | undefined;
  let old = 0;
  let next = 0;
  let inHunk = false;
  for (const line of text.replace(/\n$/, "").split("\n")) {
    const header = /^diff --git a\/(.*?) b\/(.*)$/.exec(line);
    if (header) {
      file = { path: header[2] ?? "", lines: [] };
      files.push(file);
      inHunk = false;
      continue;
    }
    if (!inHunk && /^(---|\+\+\+) /.test(line)) {
      const path = line.slice(4).trim().replace(/\t.*$/, "");
      if (path !== "/dev/null" && line.startsWith("+++")) {
        if (!file) files.push((file = { path: "", lines: [] }));
        file.path = path.replace(/^[ab]\//, "");
      }
      continue;
    }
    const hunk = HUNK.exec(line);
    if (hunk) {
      if (!file) files.push((file = { path: "", lines: [] }));
      old = Number(hunk[1]);
      next = Number(hunk[2]);
      inHunk = true;
      file.lines.push({ kind: "hunk", text: line });
      continue;
    }
    if (!file || !inHunk) continue;
    if (line.startsWith("+")) file.lines.push({ kind: "add", text: line.slice(1), new: next++ });
    else if (line.startsWith("-"))
      file.lines.push({ kind: "remove", text: line.slice(1), old: old++ });
    else if (line.startsWith(" ") || line === "") {
      file.lines.push({ kind: "context", text: line.slice(1), old: old++, new: next++ });
    }
  }
  return files.filter((f) => f.lines.length > 0);
}

function gitDiff(output: string): Component | undefined {
  const files = parseUnified(output);
  if (files.length === 0) return undefined;
  const diffs: Component[] = files.map((f) => ({ type: "diff", path: f.path, lines: f.lines }));
  return diffs.length === 1 && diffs[0] ? diffs[0] : { type: "stack", gap: "s", children: diffs };
}

const STATUS: Record<string, string> = {
  M: "modified",
  A: "added",
  D: "deleted",
  R: "renamed",
  C: "added",
  U: "modified",
  T: "modified",
};

/** The long form's words for a change. */
const LONG_STATUS: Record<string, string> = {
  "new file": "added",
  deleted: "deleted",
  renamed: "renamed",
};

/** `git status`: the short form (`XY path`) or the long one (`\tmodified:   path`, untracked files). */
function gitStatus(output: string): Component | undefined {
  const files: UiNode[] = [];
  const short = output.split("\n").filter((l) => l.trim() !== "" && !l.startsWith("##"));
  if (short.length > 0 && short.every((l) => /^[ MADRCUT?!]{2} \S/.test(l))) {
    for (const l of short) {
      const xy = l.slice(0, 2);
      const path = l.slice(3).split(" -> ").pop() ?? "";
      const code = xy === "??" ? "?" : (xy.trim()[0] ?? "M");
      files.push(
        node("File", { path, status: code === "?" ? "untracked" : (STATUS[code] ?? "modified") }),
      );
    }
  } else {
    let untracked = false;
    for (const l of output.split("\n")) {
      if (/^Untracked files:/.test(l)) untracked = true;
      else if (/^\S/.test(l)) untracked = false;
      const change = /^\t(modified|new file|deleted|renamed|typechange):\s+(.+)$/.exec(l);
      if (change?.[2]) {
        const status = LONG_STATUS[change[1] ?? ""] ?? "modified";
        files.push(node("File", { path: change[2].split(" -> ").pop() ?? change[2], status }));
      } else if (untracked && /^\t\S/.test(l)) {
        files.push(node("File", { path: l.trim(), status: "untracked" }));
      }
    }
  }
  if (files.length === 0) return undefined;
  return doc(node("FileTree", { files }));
}

/** `git log`: `--oneline` lines, or the default `commit sha / Author / Date / message` blocks. */
function gitLog(output: string): Component | undefined {
  const commits: UiNode[] = [];
  const lines = output.split("\n").filter((l) => l.trim() !== "");
  if (lines.length > 0 && lines.every((l) => /^[*|\\/ ]*[0-9a-f]{7,40} /.test(l))) {
    for (const l of lines) {
      const m = /^[*|\\/ ]*([0-9a-f]{7,40}) (?:\([^)]*\) )?(.*)$/.exec(l);
      if (m?.[1]) commits.push(node("Commit", { sha: m[1], title: m[2] ?? "" }));
    }
  } else {
    const blocks = output.split(/^commit (?=[0-9a-f]{40})/m).slice(1);
    for (const block of blocks) {
      const sha = block.slice(0, 40);
      const author = /^Author:\s+(.*?)(?: <.*>)?$/m.exec(block)?.[1];
      const date = /^Date:\s+(.*)$/m.exec(block)?.[1]?.trim();
      const title = /^\n? {4}(\S.*)$/m.exec(block.slice(block.indexOf("\n\n")))?.[1];
      if (title === undefined) continue;
      const props: Record<string, unknown> = { sha, title };
      if (author !== undefined) props.author = author;
      if (date !== undefined) props.date = date;
      commits.push(node("Commit", props));
    }
  }
  if (commits.length === 0) return undefined;
  return doc(
    commits.length === 1 && commits[0]
      ? commits[0]
      : node("Stack", { gap: "s", children: commits }),
  );
}

// ─── gh ─────────────────────────────────────────────────────────────────────

/** A pull request or issue from `gh … --json` (its field names) as props. */
function fromJson(v: Record<string, unknown>, pr: boolean): UiNode | undefined {
  if (typeof v.number !== "number" || typeof v.title !== "string") return undefined;
  const props: Record<string, unknown> = { number: v.number, title: v.title };
  const state = typeof v.state === "string" ? v.state.toLowerCase() : undefined;
  if (state !== undefined)
    props.state = pr && v.isDraft === true && state === "open" ? "draft" : state;
  const login = (x: unknown): string | undefined =>
    typeof x === "object" && x !== null && typeof (x as { login?: unknown }).login === "string"
      ? (x as { login: string }).login
      : undefined;
  const author = login(v.author);
  if (author !== undefined) props.author = author;
  if (typeof v.url === "string") {
    props.url = v.url;
    const repo = /github\.com\/([^/]+\/[^/]+)\//.exec(v.url)?.[1];
    if (repo !== undefined) props.repo = repo;
  }
  if (pr) {
    if (typeof v.headRefName === "string") props.branch = v.headRefName;
    if (typeof v.baseRefName === "string") props.base = v.baseRefName;
    if (typeof v.additions === "number") props.added = v.additions;
    if (typeof v.deletions === "number") props.removed = v.deletions;
    const review: Record<string, string> = {
      APPROVED: "approved",
      CHANGES_REQUESTED: "changes",
      REVIEW_REQUIRED: "pending",
    };
    if (typeof v.reviewDecision === "string" && review[v.reviewDecision])
      props.review = review[v.reviewDecision];
    if (Array.isArray(v.statusCheckRollup) && v.statusCheckRollup.length > 0) {
      // A check run has a conclusion once it ended, a status before; a commit status a state.
      const states = v.statusCheckRollup.map((c) => {
        const r = c as Record<string, unknown>;
        for (const key of ["conclusion", "state", "status"]) {
          const value = r[key];
          if (typeof value === "string" && value !== "") return value.toUpperCase();
        }
        return "";
      });
      props.checks = "pending";
      if (states.every((s) => /SUCCESS|NEUTRAL|SKIPPED/.test(s))) props.checks = "passing";
      if (states.some((s) => /FAIL|ERROR|CANCEL|TIMED_OUT/.test(s))) props.checks = "failing";
    }
  } else {
    if (Array.isArray(v.labels)) {
      props.labels = v.labels
        .map((l) => (typeof l === "object" && l !== null ? (l as { name?: unknown }).name : l))
        .filter((l): l is string => typeof l === "string");
    }
    if (Array.isArray(v.comments)) props.comments = v.comments.length;
    else if (typeof v.comments === "number") props.comments = v.comments;
  }
  return node(pr ? "PullRequest" : "Issue", props);
}

/** `gh pr view` / `gh issue view` without a terminal: `key:\tvalue` lines, `--`, the body. */
function ghView(output: string, pr: boolean): Component | undefined {
  const json = ghJson(output, pr);
  if (json !== undefined) return json;
  const [head = "", ...rest] = output.split(/^--$/m);
  const fields = new Map<string, string>();
  for (const m of head.matchAll(/^([a-z-]+):\t(.*)$/gm)) fields.set(m[1] ?? "", m[2]?.trim() ?? "");
  const title = fields.get("title");
  const number = Number(fields.get("number"));
  if (title === undefined || !Number.isFinite(number)) return undefined;
  const props: Record<string, unknown> = { number, title };
  const state = fields.get("state")?.toLowerCase();
  if (state)
    props.state = pr && fields.get("draft") === "true" && state === "open" ? "draft" : state;
  const author = fields.get("author");
  if (author) props.author = author;
  const url = fields.get("url");
  if (url) {
    props.url = url;
    const repo = /github\.com\/([^/]+\/[^/]+)\//.exec(url)?.[1];
    if (repo) props.repo = repo;
  }
  if (pr) {
    const added = Number(fields.get("additions"));
    const removed = Number(fields.get("deletions"));
    if (Number.isFinite(added) && fields.has("additions")) props.added = added;
    if (Number.isFinite(removed) && fields.has("deletions")) props.removed = removed;
  } else {
    const labels = fields.get("labels");
    if (labels)
      props.labels = labels
        .split(",")
        .map((l) => l.trim())
        .filter((l) => l !== "");
    const comments = Number(fields.get("comments"));
    if (Number.isFinite(comments) && comments > 0) props.comments = comments;
  }
  const card = doc(node(pr ? "PullRequest" : "Issue", props));
  const body = rest.join("--").trim();
  if (body === "") return card;
  return {
    type: "stack",
    gap: "s",
    children: [card, { type: "text", blocks: parseMarkdown(body) }],
  };
}

/** `gh … --json` output: one pull request or issue, or a list of them. */
function ghJson(output: string, pr: boolean): Component | undefined {
  let value: unknown;
  try {
    value = JSON.parse(output);
  } catch {
    return undefined;
  }
  const items = (Array.isArray(value) ? value : [value])
    .filter((v): v is Record<string, unknown> => typeof v === "object" && v !== null)
    .map((v) => fromJson(v, pr))
    .filter((n): n is UiNode => n !== undefined);
  if (items.length === 0) return undefined;
  return doc(
    items.length === 1 && items[0] ? items[0] : node("Stack", { gap: "s", children: items }),
  );
}

/** `gh pr list` / `gh issue list` without a terminal: tab-separated rows. */
function ghList(output: string, pr: boolean): Component | undefined {
  const json = ghJson(output, pr);
  if (json !== undefined) return json;
  const rows = output
    .split("\n")
    .filter((l) => l.trim() !== "")
    .map((l) => l.split("\t"));
  if (rows.length === 0 || !rows.every((r) => r.length >= 3 && /^\d+$/.test(r[0] ?? "")))
    return undefined;
  const items = rows.map((r) => {
    const number = Number(r[0]);
    if (pr) {
      // number, title, branch, state, date
      const props: Record<string, unknown> = { number, title: r[1] ?? "", branch: r[2] ?? "" };
      const state = (r[3] ?? "").toLowerCase();
      if (state !== "") props.state = state;
      return node("PullRequest", props);
    }
    // number, state, title, labels, date
    const props: Record<string, unknown> = {
      number,
      title: r[2] ?? "",
      state: (r[1] ?? "").toLowerCase() === "closed" ? "closed" : "open",
    };
    const labels = (r[3] ?? "")
      .split(",")
      .map((l) => l.trim())
      .filter((l) => l !== "");
    if (labels.length > 0) props.labels = labels;
    return node("Issue", props);
  });
  return doc(node("Stack", { gap: "s", children: items }));
}

/** `gh pr checks` without a terminal: `name\tpass|fail|pending|skipping\tduration\turl`. */
function ghChecks(output: string): Component | undefined {
  const rows = output
    .split("\n")
    .filter((l) => l.trim() !== "")
    .map((l) => l.split("\t"));
  const status: Record<string, string> = {
    pass: "passed",
    fail: "failed",
    pending: "pending",
    skipping: "skipped",
    cancel: "failed",
  };
  if (rows.length === 0 || !rows.every((r) => r.length >= 2 && status[r[1] ?? ""] !== undefined))
    return undefined;
  const checks = rows.map((r) => {
    const props: Record<string, unknown> = { name: r[0] ?? "", status: status[r[1] ?? ""] };
    if (r[2] && r[2] !== "0") props.duration = r[2];
    if (r[3]) props.url = r[3];
    return node("Check", props);
  });
  return doc(node("Checks", { checks }));
}

// ─── Which reader for which command ─────────────────────────────────────────

const test = (name: string, parse: (output: string) => Run | undefined) => (output: string) => {
  const run = parse(output);
  return run === undefined ? undefined : testRun(name, run, output);
};

const READERS: readonly Reader[] = [
  { when: /\bcargo (?:\+\S+ )?(?:test|nextest run)\b/, read: test("cargo test", cargoTest) },
  { when: /\bbun test\b/, read: test("bun test", bunTest) },
  { when: /\bgo test\b/, read: test("go test", goTest) },
  { when: /\bpytest\b/, read: test("pytest", pytest) },
  { when: /\b(?:jest|vitest)\b|\b(?:npm|pnpm|yarn) (?:run )?test\b/, read: test("tests", jsTest) },
  { when: /\bgit (?:-C \S+ )?(?:diff|show)\b/, read: gitDiff },
  { when: /\bgit (?:-C \S+ )?status\b/, read: gitStatus },
  { when: /\bgit (?:-C \S+ )?log\b/, read: gitLog },
  { when: /\bgh pr view\b/, read: (o) => ghView(o, true) },
  { when: /\bgh issue view\b/, read: (o) => ghView(o, false) },
  { when: /\bgh pr list\b/, read: (o) => ghList(o, true) },
  { when: /\bgh issue list\b/, read: (o) => ghList(o, false) },
  { when: /\bgh pr checks\b/, read: ghChecks },
];

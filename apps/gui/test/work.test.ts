import { expect, test } from "bun:test";
import type { Component, DiffLine, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { blocksText, componentText } from "../ui/plugins/transcript/components.ts";
import { clockTime, count, duration, tailLines } from "../ui/plugins/transcript/format.ts";
import {
  changedFiles,
  classify,
  groupLabel,
  groupSummary,
  leadKind,
  isPlanCall,
  isQuestion,
  relativePath,
  stripShell,
  thoughtPreview,
  type WorkEntry,
} from "../ui/plugins/transcript/work.ts";

const CWD = "/home/me/proj";

const text = (value: string): Component => ({
  type: "text",
  blocks: [{ type: "paragraph", spans: [{ text: value }] }],
});
const code = (value: string): Component => ({ type: "code", code: value });
const lines = (added: number, removed: number): DiffLine[] => [
  { kind: "hunk", text: "@@ -1,1 +1,1 @@" },
  ...Array.from({ length: removed }, (_, i): DiffLine => ({
    kind: "remove",
    text: "x",
    old: i + 1,
  })),
  ...Array.from({ length: added }, (_, i): DiffLine => ({ kind: "add", text: "y", new: i + 1 })),
];
const diff = (path: string, added: number, removed: number): Component => ({
  type: "diff",
  path,
  lines: lines(added, removed),
});
const tool = (
  label: string,
  component: Component,
  state: ViewItem["state"] = "done",
  ref = "c-1",
): ViewItem => ({ ref, kind: "tool", label, state, component });

const entry = (
  item: ViewItem,
  facts?: Parameters<typeof classify>[1]["facts"],
  active = false,
): WorkEntry => classify(item, { facts, cwd: CWD, active });

test("durations read as seconds, then minutes, then hours", () => {
  expect(duration(0)).toBe("0s");
  expect(duration(12_900)).toBe("12s");
  expect(duration(59_999)).toBe("59s");
  expect(duration(223_000)).toBe("3m 43s");
  expect(duration(3_900_000)).toBe("1h 5m");
  expect(duration(-5)).toBe("0s");
  expect(count(1, "file")).toBe("1 file");
  expect(count(2, "file")).toBe("2 files");
  expect(clockTime(new Date(2025, 0, 2, 9, 5).getTime())).toBe("09:05");
});

test("blocks and components read out as plain text", () => {
  const blocks = [
    { type: "heading" as const, level: 2, spans: [{ text: "Title" }] },
    {
      type: "list" as const,
      ordered: true,
      items: [{ blocks: [{ type: "paragraph" as const, spans: [{ text: "one" }] }] }],
    },
  ];
  expect(blocksText(blocks)).toBe("Title\n\n1. one");
  expect(componentText({ type: "stack", children: [text("a"), code("b\n")] })).toBe("a\nb");
  expect(componentText({ type: "kv", items: [{ key: "k", value: "v" }] })).toBe("k: v");
});

test("shell wrappers come off a command", () => {
  expect(stripShell("bash -lc 'ls -la'")).toBe("ls -la");
  expect(stripShell('/bin/zsh -c "echo hi && ls"')).toBe("echo hi && ls");
  expect(stripShell("sh -c echo")).toBe("echo");
  expect(stripShell("  npm test  ")).toBe("npm test");
  expect(stripShell("bashful run")).toBe("bashful run");
});

test("paths are shown relative to the session's directory", () => {
  expect(relativePath(`${CWD}/src/a.ts`, CWD)).toBe("src/a.ts");
  expect(relativePath("/etc/hosts", CWD)).toBe("/etc/hosts");
  expect(relativePath(`${CWD}x/a.ts`, CWD)).toBe(`${CWD}x/a.ts`);
});

test("a command is its text, without the shell, and its output follows", () => {
  const item = tool("bash: ls", code("a.txt\nb.txt"));
  const made = entry(item, {
    kind: "execute",
    tool: "bash",
    rawInput: { command: "bash -lc 'ls'" },
  });
  expect(made).toMatchObject({
    kind: "command",
    command: "ls",
    label: "ls",
    output: "a.txt\nb.txt",
  });
  expect(made.exit).toBeUndefined();
});

test("a running command says so, and a failed one gives its exit code", () => {
  const running = entry(
    tool("bash: npm test", { type: "stack", children: [] }, "running"),
    { kind: "execute", tool: "bash", rawInput: { command: ["bash", "-lc", "npm test"] } },
    true,
  );
  expect(running).toMatchObject({ kind: "command", live: true, label: "Running npm test" });
  const failed = entry(tool("bash: false", code("oops\nExit code 2"), "failed"), {
    kind: "execute",
    rawInput: { command: "false" },
  });
  expect(failed).toMatchObject({ failed: true, exit: 2 });
  const kv: Component = {
    type: "kv",
    items: [
      { key: "command", value: "make" },
      { key: "exit code", value: "0" },
    ],
  };
  expect(entry(tool("run", kv), { kind: "execute" })).toMatchObject({ command: "make", exit: 0 });
});

test("a pending command is not live once its turn is over", () => {
  const item = tool("bash: sleep 9", code(""), "pending");
  expect(entry(item, { kind: "execute" }, true).live).toBe(true);
  expect(entry(item, { kind: "execute" }, false).live).toBe(false);
});

test("without the harness's facts the title decides", () => {
  expect(entry(tool("bash: git status", code("clean")))).toMatchObject({
    kind: "command",
    command: "git status",
  });
  expect(entry(tool("read src/a.ts", code("x")))).toMatchObject({
    kind: "read",
    label: "Read src/a.ts",
  });
  expect(entry(tool("search: scrollbar-gutter", code("x")))).toMatchObject({ kind: "search" });
  expect(entry(tool("fetch https://a.b", code("x")))).toMatchObject({ kind: "web" });
  expect(entry(tool("something odd", code("x")))).toMatchObject({
    kind: "tool",
    label: "something odd",
  });
});

test("a guest's name in front of a title does not hide what it is", () => {
  const made = entry(tool("claude: edit new.txt", diff(`${CWD}/new.txt`, 1, 0)), {
    kind: "edit",
    agent: "claude",
  });
  expect(made).toMatchObject({ kind: "edit", label: "Changed 1 file" });
  expect(entry(tool("claude: edit new.txt", { type: "stack", children: [] }))).toMatchObject({
    kind: "edit",
  });
  expect(entry(tool("claude: bash: ls", code("")), { agent: "claude" })).toMatchObject({
    kind: "command",
    command: "ls",
  });
});

test("reads list their paths, the first in the label", () => {
  const one = entry(tool("read a.ts", code("x")), {
    kind: "read",
    rawInput: { path: `${CWD}/a.ts` },
  });
  expect(one).toMatchObject({ kind: "read", paths: ["a.ts"], label: "Read a.ts" });
  const many = entry(tool("read", code("x")), {
    kind: "read",
    rawInput: { file_path: `${CWD}/a.ts`, paths: ["b.ts", "c.ts"] },
  });
  expect(many).toMatchObject({ paths: ["a.ts", "b.ts", "c.ts"], label: "Read a.ts (+2 more)" });
});

test("an edit counts the files its diffs name, with their changes", () => {
  const made = entry(
    tool("edit a.txt", {
      type: "stack",
      children: [
        code("Successfully replaced"),
        diff(`${CWD}/a.txt`, 2, 1),
        diff(`${CWD}/a.txt`, 1, 0),
      ],
    }),
    { kind: "edit", tool: "edit" },
  );
  expect(made.kind).toBe("edit");
  expect(made.label).toBe("Changed 1 file");
  expect(made.files).toHaveLength(1);
  expect(made.files?.[0]).toMatchObject({ path: "a.txt", added: 3, removed: 1 });
  expect(made.files?.[0]?.diff?.lines).toHaveLength(4 + 2);
});

test("an edit that has not said what it changed names the file it was asked to", () => {
  const made = entry(tool("write b.ts", { type: "stack", children: [] }, "running"), {
    kind: "edit",
    rawInput: { path: `${CWD}/b.ts`, content: "…" },
  });
  expect(made.files).toEqual([{ path: "b.ts", added: 0, removed: 0 }]);
  expect(made.label).toBe("Changed 1 file");
});

test("searches say what for and where, the web is a globe", () => {
  expect(
    entry(tool("grep", code("")), {
      kind: "search",
      tool: "grep",
      rawInput: { pattern: "TODO", path: `${CWD}/src` },
    }).label,
  ).toBe("Searched TODO in src");
  expect(entry(tool("find", code("")), { kind: "search", rawInput: { query: "x" } }).label).toBe(
    "Searched x",
  );
  expect(entry(tool("search", code("")), { kind: "search" }).label).toBe("Searched code");
  const web = entry(tool("search: rust news", code("")), { kind: "search", tool: "web_search" });
  expect(web).toMatchObject({ kind: "web", label: "Searched the web for rust news" });
  expect(
    entry(tool("fetch", code("")), { kind: "fetch", rawInput: { url: "https://a.b/c" } }).label,
  ).toBe("Fetched https://a.b/c");
});

test("a plan is an event row with its lines", () => {
  const item: ViewItem = {
    ref: "plan",
    kind: "tool",
    label: "Plan",
    state: "running",
    component: {
      type: "text",
      blocks: [
        {
          type: "list",
          ordered: false,
          items: [
            { checked: true, blocks: [{ type: "paragraph", spans: [{ text: "Read it" }] }] },
            {
              checked: false,
              blocks: [{ type: "paragraph", spans: [{ text: "◐ Write it", bold: true }] }],
            },
            { checked: false, blocks: [{ type: "paragraph", spans: [{ text: "Test it" }] }] },
          ],
        },
      ],
    },
  };
  expect(entry(item)).toMatchObject({
    kind: "plan",
    label: "Plan updated",
    plan: [
      { text: "Read it", done: true, active: false },
      { text: "Write it", done: false, active: true },
      { text: "Test it", done: false, active: false },
    ],
  });
  expect(entry({ ...item, ref: "plan-claude", label: "claude: Plan" }).label).toBe(
    "claude: Plan updated",
  );
});

test("the call that wrote a plan is not a row of its own", () => {
  const call = tool("update todos", text("ok"));
  expect(isPlanCall(call, { kind: "think", tool: "todo_write" })).toBe(true);
  expect(isPlanCall(call, undefined)).toBe(true);
  expect(isPlanCall(tool("bash: ls", code("")), { kind: "execute" })).toBe(false);
  expect(isPlanCall({ ...call, ref: "plan" }, undefined)).toBe(false);
});

test("a delegation is a subagent with a status", () => {
  const made = entry(
    tool("pi · Write the palette", text("done"), "running"),
    {
      kind: "think",
      tool: "delegate",
    },
    true,
  );
  expect(made).toMatchObject({ kind: "agent", label: "pi · Write the palette", status: "running" });
});

test("a tool nobody knows is a wrench with its title and input", () => {
  const made = entry(tool("figma.export", text("3 frames")), {
    kind: "other",
    tool: "figma.export",
    rawInput: { frame: "a" },
  });
  expect(made).toMatchObject({ kind: "tool", label: "figma.export", output: "3 frames" });
  expect(made.input).toBe('{\n  "frame": "a"\n}');
  expect(entry(tool("", text("")))).toMatchObject({ kind: "tool", label: "Tool call" });
});

test("questions are recognised by their ref and by their buttons", () => {
  const button: Component = { type: "button", label: "Allow", action: "suite.permit" };
  expect(isQuestion(tool("claude asks: rm", button, "pending", "p-1"))).toBe(true);
  expect(isQuestion(tool("Mode", text("Auto"), "done", "q-2"))).toBe(true);
  expect(isQuestion(tool("x", { type: "row", children: [button] }))).toBe(true);
  expect(isQuestion(tool("bash: ls", code("")))).toBe(false);
  expect(entry(tool("Mode", text("Auto"), "done", "q-2"))).toMatchObject({
    kind: "question",
    label: "Mode",
  });
});

test("a thought is a line: its heading, else its first sentence", () => {
  expect(thoughtPreview(text("Checking the palette. Then more."))).toBe("Checking the palette.");
  expect(
    thoughtPreview({
      type: "text",
      blocks: [
        { type: "heading", level: 3, spans: [{ text: "Planning the move" }] },
        { type: "paragraph", spans: [{ text: "Body." }] },
      ],
    }),
  ).toBe("Planning the move");
  expect(thoughtPreview(text("**Bold start**\nmore"))).toBe("Bold start");
  expect(thoughtPreview(text("x".repeat(200)))?.length).toBe(100);
  expect(thoughtPreview(text(""))).toBeUndefined();
  const item: ViewItem = { ref: "t-1", kind: "thought", component: text("Hmm. Ok.") };
  expect(classify(item, { cwd: CWD, active: true, writing: true })).toMatchObject({
    kind: "thought",
    label: "Thinking",
    live: true,
  });
  expect(classify(item, { cwd: CWD, active: true })).toMatchObject({
    label: "Hmm.",
    live: false,
  });
});

/** Entries of the given kinds, with a path or label to tell them apart. */
function made(...specs: [WorkEntry["kind"], string?][]): WorkEntry[] {
  return specs.map(([kind, path], index) => {
    const item = tool(`t${index}`, code(""), "done", `c-${index}`);
    const base: WorkEntry = {
      id: item.ref,
      kind,
      item,
      label: `label ${index}`,
      live: false,
      failed: false,
    };
    if (kind === "edit") base.files = [{ path: path ?? `f${index}`, added: 1, removed: 0 }];
    if (kind === "read") base.paths = [path ?? `p${index}`];
    return base;
  });
}

test("a group says what it did, from its two main kinds of work", () => {
  expect(groupSummary(made(["command"], ["command"], ["command"]))).toBe("Ran 3 commands");
  expect(groupSummary(made(["edit", "a"], ["command"]))).toBe("Ran 1 command and changed 1 file");
  expect(
    groupSummary(made(["command"], ["command"], ["command"], ["edit", "a"], ["edit", "b"])),
  ).toBe("Ran 3 commands and changed 2 files");
  expect(groupSummary(made(["read", "a"], ["read", "a"], ["read", "b"]))).toBe("Read 2 files");
  expect(groupSummary(made(["search"], ["search"]))).toBe("Searched code 2 times");
  expect(groupSummary(made(["web"]))).toBe("Searched the web 1 time");
  expect(groupSummary(made(["tool"], ["tool"]))).toBe("Used 2 tools");
});

test("what does not fit the two main kinds is other actions", () => {
  expect(groupSummary(made(["command"], ["edit", "a"], ["read", "a"]))).toBe(
    "Ran 1 command, changed 1 file, and performed 1 other action",
  );
  expect(groupSummary(made(["command"], ["edit", "a"], ["read", "a"], ["search"], ["tool"]))).toBe(
    "Ran 1 command, changed 1 file, and performed 3 other actions",
  );
});

test("thoughts only count when nothing else was done", () => {
  expect(groupSummary(made(["thought"], ["command"]))).toBe("Ran 1 command");
  expect(groupSummary(made(["thought"], ["thought"]))).toBe("Thought");
});

test("a group's header is its one entry's label, the live entry's while it runs", () => {
  const entries = made(["command"], ["read", "a"]);
  expect(groupLabel(entries.slice(0, 1), false)).toBe("label 0");
  expect(groupLabel(entries, false)).toBe("Ran 1 command and read 1 file");
  const [command, read] = entries;
  if (command === undefined || read === undefined) throw new Error("two entries");
  expect(groupLabel([command, { ...read, live: true }], true)).toBe("label 1");
  expect(groupLabel([{ ...command, live: true }, read], true)).toBe("label 0");
});

test("a group wears the icon of its most important work", () => {
  expect(leadKind(made(["read", "a"], ["command"]))).toBe("command");
  expect(leadKind(made(["tool"], ["web"], ["thought"]))).toBe("web");
  expect(leadKind(made(["thought"]))).toBe("thought");
  expect(leadKind([])).toBe("tool");
});

test("the end of a long output is what is kept", () => {
  const text = Array.from({ length: 10 }, (_, i) => `l${i}`).join("\n");
  expect(tailLines(text, 20)).toEqual({ text, hidden: 0 });
  expect(tailLines(text, 3)).toEqual({ text: "l7\nl8\nl9", hidden: 7 });
  expect(tailLines(text, 9)).toEqual({ text, hidden: 0 });
});

test("a turn's changed files add up across its edits, with one diff each", () => {
  const entries = made(["edit", "a.ts"], ["command"], ["edit", "a.ts"], ["edit", "b.ts"]);
  for (const e of entries) {
    if (e.files)
      e.files = e.files.map((f) => ({
        ...f,
        diff: { type: "diff", path: f.path, lines: lines(1, 0) },
      }));
  }
  const files = changedFiles(entries);
  expect(files.map((f) => [f.path, f.added, f.removed])).toEqual([
    ["a.ts", 2, 0],
    ["b.ts", 1, 0],
  ]);
  expect(files[0]?.diff?.lines).toHaveLength(4);
  expect(changedFiles(made(["edit", "a.ts"]))).toEqual([]);
});

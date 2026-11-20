import { expect, test } from "bun:test";
import {
  checkCounts,
  emojiOf,
  explain,
  labelLook,
  readable,
  rowFacts,
  shortAge,
  stateWord,
  timeline,
  type Detail,
  type Summary,
} from "../ui/plugins/github/github.ts";
import { blocksOf } from "../ui/plugins/github/markdown.ts";

test("a label is drawn in its own color, lightened when too dark to read", () => {
  const red = labelLook(0xd73a4a);
  expect(red.background).toBe("rgba(215, 58, 74, 0.18)");
  const dark = labelLook(0x0e1a40);
  const light = labelLook(0xfbca04);
  // A dark label's text is lifted well above its own color; a light one's barely.
  expect(dark.color).not.toBe("rgb(14, 26, 64)");
  expect(Number(/rgb\((\d+)/.exec(dark.color)?.[1])).toBeGreaterThan(100);
  expect(Number(/rgb\((\d+)/.exec(light.color)?.[1])).toBeLessThan(255);
});

test("the state pill says why a closed issue was closed", () => {
  expect(stateWord({ state: "open" })).toBe("Open");
  expect(stateWord({ state: "merged" })).toBe("Merged");
  expect(stateWord({ state: "closed", stateReason: "COMPLETED" })).toBe("Closed");
  expect(stateWord({ state: "closed", stateReason: "NOT_PLANNED" })).toBe("Not planned");
  expect(emojiOf("heart")).toBe("❤️");
  expect(emojiOf("unknown")).toBe("unknown");
});

test("an issue's markdown becomes the chat's blocks, its template comments left out", () => {
  const blocks = blocksOf("<!-- template -->\n**Category:** x\n\n## Description\n\n```\ncode\n```");
  expect(blocks.map((block) => block.type)).toEqual(["paragraph", "heading", "code_block"]);
  expect(blocksOf("<!-- only a template -->")).toEqual([]);
});

const NOW = 1_792_162_800_000;

test("ages are short", () => {
  const ago = (secs: number): string => shortAge(NOW - secs * 1000, NOW);
  expect(ago(10)).toBe("now");
  expect(ago(300)).toBe("5m");
  expect(ago(3 * 3600)).toBe("3h");
  expect(ago(2 * 86_400)).toBe("2d");
  expect(ago(21 * 86_400)).toBe("3w");
  expect(ago(90 * 86_400)).toBe("3mo");
  expect(ago(800 * 86_400)).toBe("2y");
  expect(shortAge(0, NOW)).toBe("");
});

function detail(extra: Partial<Detail>): Detail {
  return {
    kind: "pr",
    number: 31,
    title: "Fix",
    state: "open",
    author: "bob",
    labels: [],
    body: "",
    url: "",
    createdMs: 0,
    updatedMs: 0,
    comments: [],
    reviews: [],
    reviewComments: [],
    checks: [],
    files: [],
    base: "main",
    head: "fix",
    decision: "",
    additions: 0,
    deletions: 0,
    ...extra,
  };
}

test("the conversation merges comments, reviews and line comments in time order", () => {
  const entries = timeline(
    detail({
      comments: [{ author: "alice", atMs: 3000, body: "Thanks" }],
      reviews: [
        { author: "alice", atMs: 2000, state: "CHANGES_REQUESTED", body: "Use a variable." },
        { author: "carol", atMs: 1000, state: "COMMENTED", body: "" },
      ],
      reviewComments: [
        { author: "alice", atMs: 2500, path: "a.css", line: 12, body: "here", reply: false },
        { author: "bob", atMs: 2600, path: "a.css", line: null, body: "done", reply: true },
      ],
    }),
  );
  expect(entries.map((e) => `${e.author}|${e.what}`)).toEqual([
    "alice|review changes requested",
    "alice|on a.css:12",
    "bob|reply on a.css",
    "alice|",
  ]);
});

test("checks are counted, markdown loses template comments, rows list their facts", () => {
  const counts = checkCounts([
    { name: "a", state: "pass" },
    { name: "b", state: "fail" },
    { name: "c", state: "pass" },
  ]);
  expect(counts).toEqual({ pass: 2, fail: 1, pending: 0, skipped: 0 });
  expect(readable("<!-- describe -->\r\n## What\r\nIt breaks <!-- x --> now\n<!-- unclosed")).toBe(
    "## What\nIt breaks  now",
  );
  const row: Summary = {
    number: 12,
    title: "t",
    state: "open",
    author: "alice",
    labels: ["bug", "ui"],
    comments: 3,
    updatedMs: 0,
  };
  expect(rowFacts(row)).toBe("#12 · alice · bug, ui · 3 comments");
  expect(rowFacts({ ...row, labels: [], comments: 1 })).toBe("#12 · alice · 1 comment");
  expect(explain("NotAuth", "x")).toContain("gh auth login");
  expect(explain("GhFailed", "boom")).toBe("boom");
});

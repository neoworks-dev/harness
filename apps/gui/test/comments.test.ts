import { expect, test } from "bun:test";
import {
  covers,
  formatComment,
  formatMessage,
  preview,
  rangeLabel,
  type CodeComment,
} from "../ui/lib/comments.ts";

const comment = (over: Partial<CodeComment> = {}): CodeComment => ({
  id: "c1",
  path: "src/a.ts",
  side: "new",
  start: 12,
  end: 14,
  code: ["const x = 1;", "const y = 2;", "const z = 3;"],
  diff: false,
  text: "Make these constants.",
  ...over,
});

test("a range is labelled path:Lstart-Lend, one line without the end", () => {
  expect(rangeLabel(comment())).toBe("src/a.ts:L12-L14");
  expect(rangeLabel(comment({ start: 7, end: 7 }))).toBe("src/a.ts:L7");
});

test("the preview is the first line, cut with an ellipsis", () => {
  expect(preview("  short  \nsecond", 40)).toBe("short");
  expect(preview("abcdefghij", 5)).toBe("abcd…");
});

test("a comment covers the lines of its side in its file", () => {
  const c = comment();
  expect(covers(c, "src/a.ts", "new", 12)).toBe(true);
  expect(covers(c, "src/a.ts", "new", 14)).toBe(true);
  expect(covers(c, "src/a.ts", "new", 15)).toBe(false);
  expect(covers(c, "src/a.ts", "old", 13)).toBe(false);
  expect(covers(c, "src/b.ts", "new", 13)).toBe(false);
});

test("a comment reads as the place, the quoted lines and the text", () => {
  expect(formatComment(comment())).toBe(
    [
      "`src/a.ts` lines 12-14:",
      "```ts",
      "const x = 1;",
      "const y = 2;",
      "const z = 3;",
      "```",
      "Make these constants.",
    ].join("\n"),
  );
});

test("a single line, an old version and a diff are said so", () => {
  expect(formatComment(comment({ start: 3, end: 3, code: ["a"] }))).toContain("`src/a.ts` line 3:");
  expect(formatComment(comment({ side: "old" }))).toContain(
    "lines 12-14 of the old version (before the change):",
  );
  const diff = formatComment(comment({ diff: true, code: ["-old", "+new"] }));
  expect(diff).toContain('(a diff: "+" added, "-" removed):');
  expect(diff).toContain("```diff\n-old\n+new\n```");
});

test("a fence outgrows the backticks in the code", () => {
  const out = formatComment(comment({ path: "README.md", code: ["```js", "x", "```"] }));
  expect(out).toContain("````md\n```js\nx\n```\n````");
});

test("the message is the text, then every comment; comments alone are a message too", () => {
  const two = [
    comment(),
    comment({ id: "c2", path: "b.go", start: 1, end: 1, code: ["x"], text: "Why?" }),
  ];
  const message = formatMessage("Please fix.", two);
  expect(
    message.startsWith("Please fix.\n\nComments on the code:\n\n`src/a.ts` lines 12-14:"),
  ).toBe(true);
  expect(message).toContain("Make these constants.\n\n`b.go` line 1:\n```go\nx\n```\nWhy?");
  expect(formatMessage("", two).startsWith("Comments on the code:\n\n")).toBe(true);
  expect(formatMessage("plain", [])).toBe("plain");
});

import { expect, test } from "bun:test";
import {
  committedMessage,
  errorText,
  folderName,
  preferredEditor,
  primaryAction,
  shortCount,
  sumStat,
  type Overview,
} from "../ui/plugins/workspace/workspace.ts";

test("numstat and untracked files add up to the card's counts", () => {
  expect(sumStat("", [])).toEqual({ files: 0, added: 0, deleted: 0 });
  const numstat = "2\t1\ta.txt\n10\t0\tsrc/b.ts\n-\t-\timage.png\n";
  expect(sumStat(numstat, [3, 0])).toEqual({ files: 5, added: 15, deleted: 1 });
  // Untracked files alone.
  expect(sumStat("", [4])).toEqual({ files: 1, added: 4, deleted: 0 });
});

test("counts are shortened to fit a row", () => {
  expect(shortCount(0)).toBe("0");
  expect(shortCount(999)).toBe("999");
  expect(shortCount(1000)).toBe("1k");
  expect(shortCount(1234)).toBe("1.2k");
  expect(shortCount(15_400)).toBe("15k");
});

test("the folder is the last part of the path", () => {
  expect(folderName("/home/me/code/app")).toBe("app");
  expect(folderName("/home/me/code/app/")).toBe("app");
  expect(folderName("/")).toBe("/");
  expect(folderName("app")).toBe("app");
});

test("the last editor used opens the folder while it is installed, else the first", () => {
  const zed = { id: "zed", name: "Zed" };
  const code = { id: "code", name: "VS Code" };
  expect(preferredEditor([zed, code], "code")).toBe(code);
  expect(preferredEditor([zed, code], "cursor")).toBe(zed);
  expect(preferredEditor([zed, code], undefined)).toBe(zed);
  expect(preferredEditor([], "code")).toBeUndefined();
});

const overview = (change: Partial<Overview>): Overview => ({
  branch: "main",
  upstream: "origin/main",
  ahead: 0,
  dirty: false,
  numstat: "",
  untracked: [],
  ...change,
});

test("the commit button asks for a message, pushes alone, or has nothing to do", () => {
  expect(primaryAction(overview({ dirty: true })).kind).toBe("commit-push");
  expect(primaryAction(overview({ dirty: true, upstream: null })).kind).toBe("commit-push");
  expect(primaryAction(overview({ ahead: 2 }))).toEqual({ kind: "push", label: "Push" });
  expect(primaryAction(overview({ ahead: 2, upstream: null })).kind).toBe("none");
  expect(primaryAction(overview({})).kind).toBe("none");
});

test("what a commit did is told in a sentence", () => {
  const done = { commit: "abc1234", pushed: false, note: null, failed: false };
  expect(committedMessage(done)).toBe("Committed abc1234");
  expect(committedMessage({ ...done, pushed: true })).toBe("Committed abc1234 and pushed");
  expect(committedMessage({ ...done, commit: null, pushed: true })).toBe("Pushed");
  expect(committedMessage({ ...done, note: "Not pushed: main has no upstream branch" })).toBe(
    "Committed abc1234. Not pushed: main has no upstream branch",
  );
  expect(committedMessage({ ...done, note: "Push failed: rejected", failed: true })).toBe(
    "Committed abc1234. Push failed: rejected",
  );
});

test("an error is one line", () => {
  expect(errorText(new Error("first\nsecond"))).toBe("first");
  expect(errorText("plain")).toBe("plain");
  expect(errorText(new Error(""))).toBe("Something went wrong");
});

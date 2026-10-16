import { expect, test } from "bun:test";
import {
  COMMANDS,
  completion,
  isSlashWord,
  lookup,
  matching,
  moveSelection,
} from "../ui/lib/commands.ts";

const names = (typed: string): string[] => matching(typed).map((command) => command.name);

test("a bare slash lists every command, then the list narrows by prefix", () => {
  expect(names("/")).toHaveLength(COMMANDS.length);
  expect(names("/c")).toEqual(["clear", "claude", "codex"]);
  expect(names("/mod")).toEqual(["model", "mode"]);
  expect(names("/model")).toEqual(["model"]);
});

test("without a prefix match, names that contain the text are listed", () => {
  expect(names("/idebar")).toEqual(["sidebar"]);
  expect(names("/zzz")).toEqual([]);
});

test("a slash word has no space or newline yet", () => {
  expect(isSlashWord("/")).toBe(true);
  expect(isSlashWord("/mod")).toBe(true);
  expect(isSlashWord("/model ")).toBe(false);
  expect(isSlashWord("/a\nb")).toBe(false);
  expect(isSlashWord("hi /x")).toBe(false);
});

test("the first word names the command", () => {
  expect(lookup("/model sonnet")?.name).toBe("model");
  expect(lookup("  /help")?.local).toBe(true);
  expect(lookup("/nope")).toBeUndefined();
  expect(lookup("model")).toBeUndefined();
});

test("completing a pick leaves room for arguments", () => {
  const [clear] = matching("/cl");
  if (!clear) throw new Error("no command matches /cl");
  expect(completion(clear)).toBe("/clear ");
});

test("the highlight wraps around the list", () => {
  expect(moveSelection(0, -1, 3)).toBe(2);
  expect(moveSelection(2, 1, 3)).toBe(0);
  expect(moveSelection(1, 1, 3)).toBe(2);
  expect(moveSelection(0, 1, 0)).toBe(0);
});

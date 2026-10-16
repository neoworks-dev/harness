import { expect, test } from "bun:test";
import {
  History,
  HISTORY_LIMIT,
  historyFile,
  historyLine,
  parseHistory,
  walksHistory,
} from "../ui/plugins/composer/history.ts";

test("Up walks back, Down forward, and the draft comes back at the end", () => {
  const history = new History();
  expect(history.older("draft")).toBeUndefined();
  history.push("one");
  history.push("two");
  expect(history.push("two")).toBe(false);
  expect(history.all()).toEqual(["one", "two"]);
  expect(history.older("draft")).toBe("two");
  expect(history.browsing).toBe(true);
  expect(history.older("two")).toBe("one");
  expect(history.older("one")).toBeUndefined();
  expect(history.newer()).toBe("two");
  expect(history.newer()).toBe("draft");
  expect(history.browsing).toBe(false);
  expect(history.newer()).toBeUndefined();
});

test("typing ends the walk", () => {
  const history = new History(["a", "b"]);
  history.older("");
  history.reset();
  expect(history.older("x")).toBe("b");
});

test("only the newest entries are kept", () => {
  const history = new History();
  for (let i = 0; i < HISTORY_LIMIT + 5; i++) history.push(`entry ${i}`);
  expect(history.all()).toHaveLength(HISTORY_LIMIT);
  expect(history.all()[0]).toBe("entry 5");
});

test("a history file is one JSON string per line and survives a torn last line", () => {
  const text = `${historyLine("first")}${historyLine("multi\nline")}{"not":"a string"}\n"torn`;
  expect(parseHistory(text)).toEqual(["first", "multi\nline"]);
});

test("only plain session ids name a file", () => {
  expect(historyFile("/d", "abc-12_x")).toBe("/d/abc-12_x.jsonl");
  expect(historyFile("/d", "../etc")).toBeUndefined();
  expect(historyFile("/d", "")).toBeUndefined();
});

test("Up and Down walk the history from a short single line, or when forced", () => {
  expect(walksHistory("", 80, false)).toBe(true);
  expect(walksHistory("short", 80, false)).toBe(true);
  expect(walksHistory("a\nb", 80, false)).toBe(false);
  expect(walksHistory("x".repeat(200), 80, false)).toBe(false);
  expect(walksHistory("a\nb", 80, true)).toBe(true);
});

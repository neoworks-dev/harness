import { expect, test } from "bun:test";
import { findTerminal, terminalArgv } from "../src/open.ts";

test("each terminal is told the command its own way", () => {
  const command = ["neo-term", "--session", "s1"];
  expect(terminalArgv("kitty", command)).toEqual(["kitty", "neo-term", "--session", "s1"]);
  expect(terminalArgv("/usr/bin/ghostty", command)).toEqual([
    "/usr/bin/ghostty",
    "-e",
    "neo-term",
    "--session",
    "s1",
  ]);
  expect(terminalArgv("wezterm", command)).toEqual([
    "wezterm",
    "start",
    "--",
    "neo-term",
    "--session",
    "s1",
  ]);
  expect(terminalArgv("unknown-term", command)).toEqual([
    "unknown-term",
    "-e",
    "neo-term",
    "--session",
    "s1",
  ]);
});

test("$TERMINAL wins, else the first terminal on PATH", () => {
  expect(findTerminal({ TERMINAL: "foot" }, () => true)).toBe("foot");
  expect(findTerminal({}, (name) => name === "kitty" || name === "foot")).toBe("kitty");
  expect(findTerminal({}, () => false)).toBeUndefined();
});

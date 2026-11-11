import { expect, test } from "bun:test";
import type { Component, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { OUTPUT_LINES, plainOutput, terminalRuns } from "../ui/plugins/terminal/runs.ts";
import { emptyTimeline, type Timeline } from "../ui/plugins/transcript/timeline.ts";

const code = (value: string): Component => ({ type: "code", code: value });
const tool = (ref: string, label: string, component: Component, state: ViewItem["state"] = "done"): ViewItem => ({
  ref,
  kind: "tool",
  label,
  state,
  component,
});

/** A timeline that knows each call's command and when it was heard of. */
function timeline(commands: Record<string, string>, agent?: string): Timeline {
  const t = emptyTimeline();
  for (const [ref, command] of Object.entries(commands)) {
    t.facts.set(ref, { kind: "execute", tool: "bash", rawInput: { command }, ...(agent ? { agent } : {}) });
    t.stamps.set(ref, { first: 1000, last: 4000 });
  }
  return t;
}

test("the commands are the session's execute calls, oldest first, with how each ended", () => {
  const items: ViewItem[] = [
    tool("c-1", "bash: ls", code("a\nb")),
    { ref: "m-1", kind: "agent", component: { type: "text", blocks: [] } },
    tool("c-2", "read", code("x")),
    tool("c-3", "bash: false", code("oops\nExit code 2"), "failed"),
    tool("c-4", "bash: npm test", code("running"), "running"),
  ];
  const t = timeline({ "c-1": "ls", "c-3": "false", "c-4": "npm test" });
  t.facts.set("c-2", { kind: "read", tool: "read" });
  const runs = terminalRuns({ items, timeline: t, cwd: "/w", running: true });
  expect(runs.map((r) => [r.id, r.command, r.state])).toEqual([
    ["c-1", "ls", "done"],
    ["c-3", "false", "failed"],
    ["c-4", "npm test", "running"],
  ]);
  expect(runs[0]).toMatchObject({ output: "a\nb", startedAt: 1000, endedAt: 4000 });
  expect(runs[1]?.exit).toBe(2);
  expect(runs[2]?.endedAt).toBeUndefined();
});

test("a command left pending by a finished turn isn't running, and a guest's says whose it is", () => {
  const items = [tool("c-1", "bash: make", code(""), "pending")];
  const [run] = terminalRuns({ items, timeline: timeline({ "c-1": "make" }, "codex"), cwd: "/w", running: false });
  expect(run).toMatchObject({ state: "done", agent: "codex" });
});

test("output loses its colour codes and what a carriage return wrote over, and keeps its end", () => {
  expect(plainOutput("\x1b[31mred\x1b[0m ok\n10%\r50%\r100%\n\x1b]0;title\x07done")).toBe("red ok\n100%\ndone");
  const long = Array.from({ length: OUTPUT_LINES + 10 }, (_, i) => `line ${i}`).join("\n");
  const [run] = terminalRuns({
    items: [tool("c-1", "bash: seq", code(long))],
    timeline: timeline({ "c-1": "seq" }),
    cwd: "/w",
    running: false,
  });
  expect(run?.hidden).toBe(10);
  expect(run?.output.split("\n")[0]).toBe("line 10");
});

// The commands the agent and its guests ran in the session (`execute` tool calls), as the terminal
// panel shows them: what was run and what for, what it printed (the end of it, its colour codes
// taken out) and how it ended. Read from the chat's items the way its work rows read them
// (transcript/work.ts). Plain functions, tested without a window.
import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
import { tailLines } from "../transcript/format.ts";
import type { Timeline } from "../transcript/timeline.ts";
import { classify } from "../transcript/work.ts";

/** Commands kept, the newest; and lines of output per command, its end. */
export const RUNS = 50;
export const OUTPUT_LINES = 400;

export interface TerminalRun {
  /** The item's ref. */
  id: string;
  command: string;
  /** What the agent said it is for. */
  description?: string;
  /** The guest that ran it. */
  agent?: string;
  output: string;
  /** Earlier lines of output left out. */
  hidden: number;
  state: "running" | "done" | "failed";
  exit?: number;
  /** When it started and when it was last heard of, in Unix milliseconds. */
  startedAt?: number;
  endedAt?: number;
}

// CSI and OSC sequences (colours, the cursor, titles) and the other two-character escapes.
// eslint-disable-next-line no-control-regex
const ESCAPES = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])/g;

/** The text without its terminal escapes, and a line written over with `\r` as what it ended as. */
export function plainOutput(text: string): string {
  return text
    .replace(ESCAPES, "")
    .split("\n")
    .map((line) => {
      const trimmed = line.replace(/\r+$/, "");
      return trimmed.slice(trimmed.lastIndexOf("\r") + 1);
    })
    .join("\n");
}

export interface RunsInput {
  items: readonly ViewItem[];
  timeline: Timeline;
  cwd: string;
  /** Whether a turn is running: a pending call of a finished one is not. */
  running: boolean;
}

/** The session's commands, oldest first, the last `RUNS` of them. */
export function terminalRuns({ items, timeline, cwd, running }: RunsInput): TerminalRun[] {
  const runs: TerminalRun[] = [];
  for (const item of items) {
    if (item.kind !== "tool") continue;
    const facts = timeline.facts.get(item.ref);
    const entry = classify(item, { facts, cwd, active: running });
    if (entry.kind !== "command") continue;
    const output = tailLines(plainOutput(entry.output ?? ""), OUTPUT_LINES);
    const stamp = timeline.stamps.get(item.ref);
    let state: TerminalRun["state"] = "done";
    if (entry.live) state = "running";
    else if (entry.failed || (entry.exit !== undefined && entry.exit !== 0)) state = "failed";
    const run: TerminalRun = {
      id: item.ref,
      command: entry.command ?? item.command ?? "",
      output: output.text,
      hidden: output.hidden,
      state,
    };
    if (item.description) run.description = item.description;
    if (facts?.agent !== undefined) run.agent = facts.agent;
    if (entry.exit !== undefined) run.exit = entry.exit;
    if (stamp !== undefined) {
      run.startedAt = stamp.first;
      if (state !== "running") run.endedAt = stamp.last;
    }
    runs.push(run);
  }
  return runs.slice(-RUNS);
}

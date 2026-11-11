import { expect, test } from "bun:test";
import {
  applyChunk,
  backgroundOutput,
  isActive,
  isListed,
  lastLine,
  LINGER_MS,
  newJob,
} from "../ui/plugins/terminal/jobs.ts";
import { withJobs, type TerminalRun } from "../ui/plugins/terminal/runs.ts";

const PATH = "/tmp/claude/tasks/b1.output";

test("a background command's result names its output file", () => {
  const said = `Command running in background with ID: b1. Output is being written to: ${PATH}.`;
  expect(backgroundOutput(said)).toBe(PATH);
  expect(backgroundOutput("Output is being written to: /x.output")).toBeUndefined();
  expect(backgroundOutput("running in background. Output is being written to: rel.output")).toBeUndefined();
});

test("the file is read on as it grows, a line not ended waits, and its last line says how it ended", () => {
  let job = newJob(PATH, 0);
  job = applyChunk(job, { text: "build", offset: 5, missing: false }, 100);
  expect(job).toMatchObject({ lines: [], partial: "build", seenRunning: true });
  job = applyChunk(job, { text: "ing\n\x1b[32mok\x1b[0m\n", offset: 15, missing: false }, 200);
  expect(job.lines).toEqual(["building", "ok"]);
  expect(isActive(job)).toBe(true);
  expect(isListed(job, 200)).toBe(true);
  job = applyChunk(job, { text: "\n[exited with code 2]\n", offset: 38, missing: false }, 300);
  expect(job).toMatchObject({ exit: 2, endedAt: 300, lines: ["building", "ok"] });
  expect(lastLine(job)).toBe("ok");
  expect(isActive(job)).toBe(false);
  expect(isListed(job, 300 + LINGER_MS - 1)).toBe(true);
  expect(isListed(job, 300 + LINGER_MS)).toBe(false);
});

test("one found ended at its first read was replayed, and isn't listed", () => {
  const job = applyChunk(newJob(PATH, 0), { text: "done\n[exited with code 0]\n", offset: 26, missing: false }, 10);
  expect(job.exit).toBe(0);
  expect(isListed(job, 10)).toBe(false);
});

test("a run in the background takes its output and state from its file", () => {
  const base: TerminalRun = { id: "c-1", command: "npm run dev", output: "", hidden: 0, state: "done", job: PATH, endedAt: 50 };
  const plain: TerminalRun = { id: "c-2", command: "ls", output: "a", hidden: 0, state: "done" };
  const running = applyChunk(newJob(PATH, 0), { text: "listening\n", offset: 10, missing: false }, 10);
  expect(withJobs([base, plain], { "c-1": running })).toEqual([
    { id: "c-1", command: "npm run dev", output: "listening", hidden: 0, state: "running", job: PATH },
    plain,
  ]);
  const ended = applyChunk(running, { text: "[exited with code 1]\n", offset: 31, missing: false }, 99);
  expect(withJobs([base], { "c-1": ended })[0]).toMatchObject({ state: "failed", exit: 1, endedAt: 99 });
  const killed = applyChunk(running, { text: "[killed]\n", offset: 19, missing: false }, 99);
  expect(withJobs([base], { "c-1": killed })).toEqual([]);
  // Not read yet: as the call said.
  expect(withJobs([base], {})).toEqual([base]);
});

test("a stopped one is dropped, and a missing file is waited for a while, then gone", () => {
  const killed = applyChunk(newJob(PATH, 0), { text: "x\n[killed]\n", offset: 11, missing: false }, 10);
  expect(killed.killed).toBe(true);
  expect(isListed(killed, 10)).toBe(false);
  const waiting = applyChunk(newJob(PATH, 0), { text: "", offset: 0, missing: true }, 5000);
  expect(waiting.gone).toBe(false);
  expect(applyChunk(waiting, { text: "", offset: 0, missing: true }, 10_001).gone).toBe(true);
});

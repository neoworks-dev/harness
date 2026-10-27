import { expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BackgroundCalls, createBashTool } from "../src/tools/bash.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

async function run(cwd: string, command: string, timeout?: number): Promise<string> {
  return resultText(
    await createBashTool(cwd).execute("t", { command, description: "test", timeout }),
  );
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

test("runs in the session cwd and combines stdout and stderr", async () => {
  const cwd = await tempDir();
  expect(await run(cwd, "pwd")).toBe(cwd);
  const both = await run(cwd, "echo out; echo err >&2");
  expect(both).toContain("out");
  expect(both).toContain("err");
  expect(await run(cwd, "true")).toBe("(no output)");
});

test("a non-zero exit is an error that keeps the output and the exit code", async () => {
  const message = await errorMessage(run(await tempDir(), "echo partial; exit 3"));
  expect(message).toContain("partial");
  expect(message).toContain("exited with code 3");
});

test("keeps the tail of large output and saves the full output to a file", async () => {
  const cwd = await tempDir();
  const result = await createBashTool(cwd).execute("t", {
    command: "seq 1 5000",
    description: "test",
  });
  const text = resultText(result);
  const path = result.details.fullOutputPath;
  expect(path).toBeDefined();
  expect(text).toContain("5000");
  expect(text).not.toMatch(/^1$/m);
  expect(text).toContain(`Full output: ${path}`);
  expect(text.split("\n").length).toBeLessThan(2010);
  expect(readFileSync(path ?? "", "utf8").startsWith("1\n2\n")).toBe(true);
  expect(
    readFileSync(path ?? "", "utf8")
      .trimEnd()
      .endsWith("5000"),
  ).toBe(true);
});

test("a timeout kills the command and reports it", async () => {
  const started = Date.now();
  const message = await errorMessage(run(await tempDir(), "echo before; sleep 30", 0.3));
  expect(message).toContain("before");
  expect(message).toContain("timed out after 0.3 seconds");
  expect(Date.now() - started).toBeLessThan(5000);
});

test("abort kills the whole process group", async () => {
  const cwd = await tempDir();
  const controller = new AbortController();
  const tool = createBashTool(cwd);
  const pidFile = join(cwd, "pid");
  const running = tool.execute(
    "t",
    { command: `sleep 30 & echo $! > ${pidFile}; wait`, description: "test" },
    controller.signal,
  );
  for (let i = 0; i < 100 && !existsSync(pidFile); i++) await Bun.sleep(20);
  const pid = Number(readFileSync(pidFile, "utf8"));
  expect(alive(pid)).toBe(true);

  controller.abort();
  expect(await errorMessage(running)).toContain("aborted");
  for (let i = 0; i < 100 && alive(pid); i++) await Bun.sleep(20);
  expect(alive(pid)).toBe(false);
});

test("an already aborted signal never starts the command", async () => {
  const cwd = await tempDir();
  const controller = new AbortController();
  controller.abort();
  const tool = createBashTool(cwd);
  await errorMessage(
    tool.execute("t", { command: "touch ran", description: "test" }, controller.signal),
  );
  expect(existsSync(join(cwd, "ran"))).toBe(false);
});

test("streams throttled partial output", async () => {
  const updates: string[] = [];
  const tool = createBashTool(await tempDir());
  await tool.execute(
    "t",
    { command: "for i in 1 2 3 4; do echo line$i; sleep 0.15; done", description: "test" },
    undefined,
    (partial) => updates.push(resultText(partial)),
  );
  expect(updates.length).toBeGreaterThanOrEqual(2);
  expect(updates.length).toBeLessThanOrEqual(4);
  expect(updates[0]).toContain("line1");
  expect(updates.at(-1)).toContain("line3");
});

test("a background process holding the pipes does not hang the call", async () => {
  const started = Date.now();
  const text = await run(await tempDir(), "(sleep 3 &) ; echo done");
  expect(text).toBe("done");
  expect(Date.now() - started).toBeLessThan(2000);
});

test("sent to the background, the call ends at once and the command runs on into its file", async () => {
  const cwd = await tempDir();
  const told: string[] = [];
  const background = new BackgroundCalls((text) => told.push(text));
  const aborter = new AbortController();
  const call = createBashTool(cwd, undefined, undefined, background).execute(
    "t1",
    { command: "echo first; sleep 0.4; echo second; exit 4", description: "test" },
    aborter.signal,
  );
  await Bun.sleep(150);
  expect(background.move("nope")).toBe(false);
  expect(background.move("t1")).toBe(true);
  const text = resultText(await call);
  const path = /Output is being written to: (\S+)/.exec(text)?.[1] ?? "";
  // The words term looks for to tail the file as a background command.
  expect(text).toContain("running in background");
  expect(text).toContain("first");
  // A cancelled turn no longer stops it; it ends its file with how it ended.
  aborter.abort();
  await Bun.sleep(600);
  expect(readFileSync(path, "utf8")).toBe("first\nsecond\n\n[exited with code 4]\n");
  // The agent hears how it ended, with the end of its output.
  expect(told).toEqual([
    `The command "test" you ran, which the user sent to the background, exited with code 4. The end of its output:\nfirst\nsecond\n\nAll of it: ${path}`,
  ]);
  // Only while it runs.
  expect(background.move("t1")).toBe(false);
});

test("a command run to its end leaves no output file behind", async () => {
  const cwd = await tempDir();
  const before = new Set(readdirSync(tmpdir()).filter((f) => f.startsWith("neo-bash-")));
  expect(await run(cwd, "echo hi")).toBe("hi");
  const after = readdirSync(tmpdir()).filter((f) => f.startsWith("neo-bash-") && !before.has(f));
  expect(after).toEqual([]);
});

// The service as the daemon runs it: a tool file written while it runs shows up as tools.<name>
// in the gateway, and a neoui doc that polls it follows the file as it changes.
import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, realpathSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Subprocess } from "bun";
import { startTestDaemon, type TestDaemon } from "../../../daemon/src/testing.ts";
import { fingerprint } from "../src/approval.ts";
import { Program } from "@neoworks/neoui";

const cleanups: (() => Promise<void> | void)[] = [];
afterAll(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function until<T>(
  what: string,
  check: () => Promise<T | undefined> | T | undefined,
): Promise<T> {
  const end = Date.now() + 10_000;
  while (Date.now() < end) {
    const found = await check();
    if (found !== undefined) return found;
    await Bun.sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** The service in a fresh folder; as a project's it is told the folder and its app name, as the daemon does. */
async function start(
  project: false | "open" | "gated" = false,
): Promise<{ daemon: TestDaemon; dir: string; proc: Subprocess; approved: string }> {
  const daemon = await startTestDaemon();
  cleanups.push(() => daemon.stop());
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "tools-service-")));
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
  // Where the daemon would keep the hashes of the files the user allowed.
  const approvals = realpathSync(mkdtempSync(join(tmpdir(), "tools-approved-")));
  cleanups.push(() => rmSync(approvals, { recursive: true, force: true }));
  const approved = join(approvals, "tools-demo-1234.json");
  const args: string[] = [];
  if (project) args.push("--dir", dir, "--app", "tools-demo-1234");
  if (project === "gated") {
    writeFileSync(approved, "{}");
    args.push("--approved", approved);
  }
  const proc = Bun.spawn(
    [process.execPath, "run", join(import.meta.dir, "..", "main.ts"), ...args],
    {
      env: { ...process.env, ...daemon.env, NEO_TOOLS_DIR: dir },
      stdin: "ignore",
      stdout: "ignore",
      stderr: "ignore",
    },
  );
  cleanups.push(() => proc.kill());
  return { daemon, dir, proc, approved };
}

interface Called {
  isError: boolean;
  text: string;
  data: Record<string, unknown> | undefined;
}

const call = async (
  daemon: TestDaemon,
  name: string,
  args: Record<string, unknown> = {},
): Promise<Called> => {
  const result = await daemon.gateway.call("tools_call", { name, args });
  const lines: string[] = [];
  for (const c of result.content) if (c.type === "text") lines.push(c.text);
  return {
    isError: result.isError === true,
    text: lines.join("\n"),
    data: result.structuredContent,
  };
};

test("a tool file written while it runs becomes a tool, and a polling doc follows it", async () => {
  const { daemon, dir } = await start();
  await until("tools.status", async () => {
    const r = await call(daemon, "tools.status").catch(() => undefined);
    return r?.isError === false ? r : undefined;
  });

  const file = join(dir, "progress.ts");
  writeFileSync(
    file,
    `let percent = 0;
export default {
  name: "progress",
  description: "A pretend download that moves 50 points a call",
  inputSchema: { type: "object", properties: {} },
  async handler() {
    percent += 50;
    return { content: [{ type: "text", text: percent + "%" }], structuredContent: { percent } };
  },
};`,
  );
  writeFileSync(join(dir, "broken.ts"), "export default {{");
  await until("tools.progress", async () => {
    const r = await call(daemon, "tools.status");
    return r.text.includes("progress.ts: progress") ? r : undefined;
  });
  const status = await call(daemon, "tools.status");
  expect(status.text).toContain("broken.ts: no tools (can't load:");

  // The doc a model would write: a bar that asks every second until it is full.
  const asked: number[] = [];
  const program = new Program("0", {
    markdown: (text) => [{ type: "paragraph", text }] as never,
    callTool: async (name, args) => {
      const r = await call(daemon, name, args);
      if (r.isError) throw new Error(r.text);
      asked.push(r.data?.percent as number);
      return r.data;
    },
  });
  program.onChange = () => program.doc();
  program.set(
    `<Stack>
      <Query id="dl" hx-get="tools.progress" hx-trigger="load, every 1s" until={dl.percent >= 100} default={{ percent: 0 }}/>
      <Progress value={dl.percent}/>
    </Stack>`,
    false,
  );
  program.doc();
  await until("a full bar", () => (asked.includes(100) ? true : undefined));
  const bar = JSON.stringify(program.doc().root);
  expect(bar).toContain('"value":100');
  // until held: no more asks.
  const seen = asked.length;
  await Bun.sleep(1300);
  expect(asked).toHaveLength(seen);
  program.dispose();
});

test("started for a project it is that project's app, on the folder it was given", async () => {
  const { daemon, dir } = await start("open");
  writeFileSync(
    join(dir, "where.ts"),
    `export default { name: "where", description: "d", inputSchema: {}, handler: async () => "here" };`,
  );
  const status = await until("the project's tools", async () => {
    const r = await call(daemon, "tools-demo-1234.status").catch(() => undefined);
    return r?.text.includes("where.ts: where") ? r : undefined;
  });
  expect(status.data?.dir).toBe(dir);
  expect((await call(daemon, "tools-demo-1234.where")).text).toBe("here");
  // The user's own instance is not this one.
  expect((await call(daemon, "tools.status")).isError).toBe(true);
});

test("with approvals a file runs once the daemon writes its hash, and stops when it changes", async () => {
  const { daemon, dir, approved } = await start("gated");
  const app = "tools-demo-1234";
  const file = join(dir, "where.ts");
  const write = (word: string): void =>
    writeFileSync(
      file,
      `export default { name: "where", description: "d", inputSchema: {}, handler: async () => "${word}" };`,
    );
  const statusText = async (): Promise<string> => {
    const r = await call(daemon, `${app}.status`).catch(() => undefined);
    return r?.text ?? "";
  };

  write("here");
  await until("the file listed as not allowed", async () => {
    return (await statusText()).includes("not allowed yet") ? true : undefined;
  });
  expect((await call(daemon, `${app}.where`)).isError).toBe(true);

  // The daemon allows it, the way it writes: whole, then renamed into place.
  const allow = (): void => {
    writeFileSync(`${approved}.part`, JSON.stringify({ "where.ts": fingerprint(dir)["where.ts"] }));
    renameSync(`${approved}.part`, approved);
  };
  allow();
  await until("the tool", async () => {
    const r = await call(daemon, `${app}.where`).catch(() => undefined);
    return r?.isError === false ? r : undefined;
  });
  expect((await call(daemon, `${app}.where`)).text).toBe("here");

  // Changed after it was allowed: it stops, until it is allowed as it is.
  write("changed");
  await until("the tool stopped", async () => {
    return (await statusText()).includes("changed since the user approved it") ? true : undefined;
  });
  expect((await call(daemon, `${app}.where`)).isError).toBe(true);
  allow();
  await until("the changed tool", async () => {
    const r = await call(daemon, `${app}.where`).catch(() => undefined);
    return r?.isError === false ? r : undefined;
  });
  expect((await call(daemon, `${app}.where`)).text).toBe("changed");
});

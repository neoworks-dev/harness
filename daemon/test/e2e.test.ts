// End to end: the real daemon (with the scripted echo agent) and the real Go CLI over the Unix
// socket. Needs bin/suite (`make build`; `make check` builds before testing).
import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { Subprocess } from "bun";
import { startMcpApp } from "./mcp-app.ts";

const repo = resolve(import.meta.dir, "..", "..");
const suiteBin = join(repo, "bin", "suite");
const daemonMain = join(repo, "daemon", "src", "main.ts");

interface Env {
  [key: string]: string;
}

interface Attached {
  proc: Subprocess<"pipe", "pipe", "pipe">;
  output(): string;
  send(line: string): void;
  waitFor(text: string, timeoutMs?: number): Promise<void>;
}

const running: Subprocess[] = [];
const dirs: string[] = [];
afterAll(() => {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});
afterEach(() => {
  for (const p of running.splice(0)) p.kill("SIGKILL");
});

function testEnv(): Env {
  const dir = mkdtempSync(join(tmpdir(), "suite-e2e-"));
  dirs.push(dir);
  return {
    ...(process.env as Env),
    SUITE_SOCKET: join(dir, "bus.sock"),
    SUITE_DATA_DIR: join(dir, "data"),
    SUITE_AGENT: "echo",
    SUITE_SERVICES: "none",
  };
}

async function until(check: () => boolean, what: string, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(10);
  }
}

async function startDaemon(env: Env): Promise<Subprocess> {
  const proc = Bun.spawn(["bun", "run", daemonMain], { env, stdout: "ignore", stderr: "pipe" });
  running.push(proc);
  // The socket file can be a stale leftover of a killed daemon, so wait until it answers.
  const deadline = Date.now() + 5000;
  for (;;) {
    const live = await fetch("http://localhost/", { unix: env.SUITE_SOCKET }).then(
      () => true,
      () => false,
    );
    if (live) return proc;
    if (Date.now() > deadline) throw new Error("daemon did not start");
    await Bun.sleep(10);
  }
}

async function suite(env: Env, args: string[], stdin = ""): Promise<{ code: number; out: string }> {
  const proc = Bun.spawn([suiteBin, ...args], { env, stdin: new Blob([stdin]), stderr: "pipe" });
  const [out, err, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  return { code, out: out + err };
}

/** `suite attach` with stdin kept open, so lines can be sent while it runs. */
function attach(env: Env, args: string[]): Attached {
  const proc = Bun.spawn([suiteBin, "attach", ...args], {
    env,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  running.push(proc);
  let out = "";
  const decoder = new TextDecoder();
  void (async () => {
    for await (const chunk of proc.stdout) out += decoder.decode(chunk);
  })();
  return {
    proc,
    output: () => out,
    send(line) {
      void proc.stdin.write(`${line}\n`);
      void proc.stdin.flush();
    },
    waitFor: (text, timeoutMs) =>
      until(() => out.includes(text), `"${text}" in\n${out}`, timeoutMs),
  };
}

function sessionIdFrom(output: string): string {
  const id = /created session (s_[a-z0-9]+)/.exec(output)?.[1];
  if (!id) throw new Error(`no session id in:\n${output}`);
  return id;
}

describe.skipIf(!existsSync(suiteBin))("daemon + CLI end to end", () => {
  test("attach --new streams a turn, including a tool call", async () => {
    const env = testEnv();
    await startDaemon(env);
    const { code, out } = await suite(env, ["attach", "--new"], "make a file [tool]\n");
    expect(code).toBe(0);
    expect(out).toContain("» cli\nmake a file [tool]");
    expect(out).toContain("✓ echo tool\n    tool output");
    expect(out).toContain("echo #1: make a file [tool]");

    const status = await suite(env, ["status"]);
    // The daemon created the current session at startup; --new made a second one.
    expect(status.out).toContain("2 total, 0 running");
  });

  test("two attached clients see the same live output and both can send input", async () => {
    const env = testEnv();
    await startDaemon(env);
    const a = attach(env, ["--new"]);
    await a.waitFor("-- attached to");
    const id = sessionIdFrom(a.output());
    const b = attach(env, [id, "--app", "overlay"]);
    await b.waitFor("-- attached to");

    a.send("from a");
    await Promise.all([a.waitFor("echo #1: from a"), b.waitFor("echo #1: from a")]);
    b.send("from b");
    await Promise.all([a.waitFor("echo #2: from b"), b.waitFor("echo #2: from b")]);
    expect(a.output()).toContain("» overlay\nfrom b");

    // Same events in the same order on both sides (b attached before anything happened).
    const live = (s: string): string => s.slice(s.indexOf("-- attached to"));
    expect(live(a.output())).toBe(live(b.output()));

    for (const c of [a, b]) void c.proc.stdin.end();
    expect(await Promise.all([a.proc.exited, b.proc.exited])).toEqual([0, 0]);
  });

  test("sessions survive kill -9 mid-turn; re-attach replays everything and continues", async () => {
    const env = testEnv();
    const first = await startDaemon(env);
    const a = attach(env, ["--new", "--title", "durable"]);
    await a.waitFor("-- attached to");
    const id = sessionIdFrom(a.output());
    a.send("first turn");
    await a.waitFor("echo #1: first turn");
    a.send("wait:60000 slow turn");
    await a.waitFor("» cli\nwait:60000 slow turn");

    first.kill("SIGKILL");
    expect(await a.proc.exited).toBe(1); // connection lost

    await startDaemon(env);
    const sessions = await suite(env, ["sessions"]);
    expect(sessions.out).toContain(id);
    expect(sessions.out).toContain("durable");

    const { code, out } = await suite(env, ["attach", id.slice(0, 6)], "after restart\n");
    expect(code).toBe(0);
    const replay = out.slice(0, out.indexOf("-- attached to"));
    expect(replay).toContain("echo #1: first turn");
    expect(replay).toContain("» cli\nwait:60000 slow turn");
    expect(replay).toContain("Interrupted: the daemon stopped during this turn.");
    // The agent was rebuilt from the log: this is its third turn.
    expect(out.slice(out.indexOf("-- attached to"))).toContain("echo #3: after restart");
  });

  test("an app registered over the TCP bus shows up in suite tools until it disconnects", async () => {
    const env = testEnv();
    await startDaemon(env);
    const app = startMcpApp({
      tools: [
        {
          name: "greet",
          description: "Say hello to someone",
          inputSchema: { type: "object", properties: { name: { type: "string" } } },
          handler: () => ({ content: [{ type: "text", text: "hi" }] }),
        },
      ],
    });
    const info: { url: string; token: string } = await Bun.file(
      join(env.SUITE_SOCKET ?? "", "..", "daemon.json"),
    ).json();
    const ws = new WebSocket(`${info.url.replace("http", "ws")}/bus?token=${info.token}`);
    const frames: string[] = [];
    ws.onmessage = (e) => frames.push(String(e.data));
    await new Promise((resolve) => (ws.onopen = resolve));
    ws.send(JSON.stringify({ topic: "bus.hello", payload: { protocol: 1, client: "test-app" } }));
    ws.send(
      JSON.stringify({
        topic: "app.presence",
        id: "p1",
        payload: { app: "greeter", kind: "app", title: "Greeter", open: true, mcp: app.endpoint },
      }),
    );
    await until(() => frames.some((f) => f.includes('"bus.ack"')), "presence ack");

    const listed = await suite(env, ["tools", "hello"]);
    expect(listed.code).toBe(0);
    expect(listed.out).toContain("greeter.greet");
    expect(listed.out).toContain("connected");

    ws.close();
    for (let i = 0; i < 50; i++) {
      if ((await suite(env, ["tools"])).out.includes("closed")) break;
      await Bun.sleep(20);
    }
    expect((await suite(env, ["tools"])).out).toContain("closed");
    await app.close();
  });

  test("/clear in attach moves to a fresh session in the same directory", async () => {
    const env = testEnv();
    await startDaemon(env);
    const { out } = await suite(env, ["attach", "--new"], "hello\n/clear\nagain\n");
    const first = sessionIdFrom(out);
    const fresh = /cleared: new session (s_[a-z0-9]+)/.exec(out)?.[1] ?? "";
    expect(fresh).not.toBe("");
    expect(fresh).not.toBe(first);
    expect(out.indexOf("echo #1: hello")).toBeLessThan(out.indexOf("cleared"));
    // The new session starts its own count.
    expect(out.slice(out.indexOf("cleared"))).toContain("echo #1: again");
    expect((await suite(env, ["focus"])).out).toContain(fresh);
  });

  test("piped inputs are queued and run in order", async () => {
    const env = testEnv();
    await startDaemon(env);
    const { out } = await suite(env, ["attach", "--new"], "one\ntwo\n");
    expect(out.indexOf("echo #1: one")).toBeLessThan(out.indexOf("echo #2: two"));
  });

  test("the daemon starts the lsp service, which leaves when the daemon is killed", async () => {
    const env = { ...testEnv(), SUITE_SERVICES: "" };
    const daemon = await startDaemon(env);
    let listed = "";
    for (let i = 0; i < 100 && !listed.includes("lsp.diagnostics"); i++) {
      listed = (await suite(env, ["tools", "diagnostics"])).out;
      await Bun.sleep(50);
    }
    expect(listed).toContain("lsp.diagnostics");
    const children = Bun.spawnSync(["pgrep", "-P", String(daemon.pid)]).stdout.toString();
    const pids = children
      .split("\n")
      .filter((line) => line !== "")
      .map(Number);
    expect(pids.length).toBeGreaterThan(0);
    daemon.kill("SIGKILL");
    const alive = (pid: number): boolean => {
      try {
        process.kill(pid, 0);
        return true;
      } catch {
        return false;
      }
    };
    await until(() => !pids.some(alive), "the lsp service to exit", 5000);
  }, 15_000);
});

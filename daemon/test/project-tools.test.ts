import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { fingerprint } from "../../services/tools/src/approval.ts";
import { silentLogger } from "../src/log.ts";
import { appOf, ProjectTools, projectOf } from "../src/project-tools.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { startMcpApp, type McpApp } from "./mcp-app.ts";

type PermissionRequest = Extract<SessionEvent, { type: "permission_request" }>;

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function tempDir(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "project-tools-")));
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/** A project whose .neo/tools holds these files (name → content); none: no folder at all. */
function project(files: Record<string, string> = { "a.ts": "export default [];" }): string {
  const dir = tempDir();
  if (Object.keys(files).length > 0) mkdirSync(join(dir, ".neo", "tools"), { recursive: true });
  for (const [name, content] of Object.entries(files)) writeTool(dir, name, content);
  return dir;
}

function writeTool(dir: string, name: string, content: string): void {
  const path = join(dir, ".neo", "tools", name);
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, content);
}

describe("which directory is the project", () => {
  test("the nearest one up from the working directory that has tool files", () => {
    const dir = project();
    mkdirSync(join(dir, "src", "deep"), { recursive: true });
    expect(projectOf(dir)).toBe(dir);
    expect(projectOf(join(dir, "src", "deep"))).toBe(dir);
  });

  test("a folder with no tool files is not a project, and neither is a directory without one", () => {
    expect(projectOf(project({ "notes.txt": "x" }))).toBeUndefined();
    expect(projectOf(project({}))).toBeUndefined();
  });

  test("its app name belongs to the directory, and is readable", () => {
    const a = appOf("/work/My App");
    expect(a).toMatch(/^tools-my-app-[0-9a-f]{4}$/);
    expect(appOf("/other/My App")).not.toBe(a);
    expect(appOf("/work/My App")).toBe(a);
  });
});

describe("allowing a project's files", () => {
  /** A ProjectTools on a fake store, whose questions are answered from `answers`. */
  function setup(
    answers: (string | undefined)[],
    on = { value: true },
  ): {
    make: () => ProjectTools;
    sessions: Map<string, { cwd: string }>;
    asked: { session: string; details: string }[];
    on: { value: boolean };
  } {
    const meta = new Map<string, string>();
    const sessions = new Map<string, { cwd: string }>();
    const asked: { session: string; details: string }[] = [];
    const next = [...answers];
    const dataDir = tempDir();
    const make = (): ProjectTools =>
      new ProjectTools({
        store: {
          getMeta: (key) => meta.get(key),
          setMeta: (key, value) => void meta.set(key, value),
          getSession: (id) => sessions.get(id),
        },
        asker: {
          ask: (session, question) => {
            const details = question.type === "permission_request" ? question.details : undefined;
            const files = details?.find((d) => d.key === "Files");
            asked.push({ session, details: files?.value ?? "" });
            return Promise.resolve(next.shift());
          },
        },
        agent: "neo",
        log: silentLogger,
        dataDir,
        enabled: () => on.value,
      });
    return { make, sessions, asked, on };
  }
  const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 15));
  const allowedFile = (tools: ProjectTools, dir: string): Record<string, string> => {
    const stored: Record<string, string> = JSON.parse(readFileSync(tools.allowedPath(dir), "utf8"));
    return stored;
  };

  test("the first session asks about the files; yes starts the service and says what it may run", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    const dir = project({ "a.ts": "// a", "b.ts": "// b" });
    sessions.set("s1", { cwd: dir });
    sessions.set("s2", { cwd: dir });
    const tools = make();
    let changes = 0;
    tools.onChange(() => changes++);
    expect(tools.commands()).toEqual([]);

    tools.check("s1");
    tools.check("s2"); // asked already, still waiting
    await settle();
    expect(asked).toEqual([{ session: "s1", details: "a.ts, b.ts" }]);
    expect(changes).toBe(1);
    const [command] = tools.commands();
    expect(command?.slice(3)).toEqual([
      "--dir",
      join(dir, ".neo", "tools"),
      "--app",
      appOf(dir),
      "--approved",
      tools.allowedPath(dir),
    ]);
    // What the service holds to: each file by its hash.
    expect(allowedFile(tools, dir)).toEqual(fingerprint(join(dir, ".neo", "tools")));

    tools.check("s2");
    await settle();
    expect(asked).toHaveLength(1);
  });

  test("a new file asks about itself only, and one that changed asks again as changed", async () => {
    const { make, sessions, asked } = setup(["allow", "allow", "allow"]);
    const dir = project({ "a.ts": "// a" });
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();

    writeTool(dir, "b.ts", "// b");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "b.ts"]);

    writeTool(dir, "a.ts", "// a, changed");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "b.ts", "a.ts (changed)"]);
    expect(allowedFile(tools, dir)).toEqual(fingerprint(join(dir, ".neo", "tools")));
  });

  test("a helper a tool imports is part of it: changing one asks about every tool file", async () => {
    const { make, sessions, asked } = setup(["allow", "allow"]);
    const dir = project({ "a.ts": "// a", "b.ts": "// b", "lib/say.ts": "export const s = 1;" });
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts, b.ts"]);

    writeTool(dir, "lib/say.ts", "export const s = 2;");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts, b.ts", "a.ts (changed), b.ts (changed)"]);
  });

  test("a file that nobody touched since it was allowed is not asked about again", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    const dir = project();
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(1);
  });

  test("the answers are kept: a restarted daemon writes what was allowed and does not ask", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    const dir = project();
    sessions.set("s", { cwd: dir });
    const first = make();
    first.check("s");
    await settle();
    rmSync(first.allowedPath(dir));

    const second = make();
    expect(second.commands()).toHaveLength(1);
    expect(allowedFile(second, dir)).toEqual(fingerprint(join(dir, ".neo", "tools")));
    second.check("s");
    await settle();
    expect(asked).toHaveLength(1);
  });

  test("no is kept for that content only; no answer asks again", async () => {
    const { make, sessions, asked } = setup([undefined, "deny", "deny"]);
    const dir = project();
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    expect(tools.commands()).toEqual([]);
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(2);
    expect(tools.commands()).toEqual([]);
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(2);

    writeTool(dir, "a.ts", "// something else");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "a.ts", "a.ts (changed)"]);
  });

  test("with the setting off nothing asks and nothing runs; on, it asks", async () => {
    const { make, sessions, asked, on } = setup(["allow"], { value: false });
    const dir = project();
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    expect(asked).toEqual([]);

    on.value = true;
    tools.check("s");
    await settle();
    expect(tools.commands()).toHaveLength(1);
    on.value = false;
    expect(tools.commands()).toEqual([]);
  });

  test("a session outside any project, or one that is gone, asks nothing", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    sessions.set("plain", { cwd: project({}) });
    const tools = make();
    tools.check("plain");
    tools.check("missing");
    await settle();
    expect(asked).toEqual([]);
  });

  test("a project whose tools are gone has nothing to start", async () => {
    const { make, sessions } = setup(["allow"]);
    const dir = project();
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    rmSync(join(dir, ".neo"), { recursive: true });
    expect(tools.commands()).toEqual([]);
  });

  test("a project's tools are seen from inside it only", () => {
    const { make, sessions } = setup([]);
    const dir = project();
    const other = project();
    sessions.set("in", { cwd: dir });
    sessions.set("deep", { cwd: join(dir, "src") });
    sessions.set("out", { cwd: other });
    const tools = make();
    tools.decide(dir, { "a.ts": "hash" }, "allow");
    const app = appOf(dir);
    expect(tools.visible(app, "in")).toBe(true);
    expect(tools.visible(app, "deep")).toBe(true);
    expect(tools.visible(app, "out")).toBe(false);
    expect(tools.visible(app, "missing")).toBe(false);
    // The bus, and every other connector, see what they always did.
    expect(tools.visible(app, undefined)).toBe(true);
    expect(tools.visible("tools", "out")).toBe(true);
    expect(tools.visible("notes", "out")).toBe(true);
    // A directory that only starts with the project's name is not inside it.
    sessions.set("sibling", { cwd: `${dir}-more` });
    expect(tools.visible(app, "sibling")).toBe(false);
  });
});

describe("when a session works in a project", () => {
  async function opened(): Promise<{ daemon: TestDaemon; dir: string; session: string }> {
    const daemon: TestDaemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const dir = project({ "progress.ts": "// first" });
    const session = daemon.sessions.create({ cwd: dir }).id;
    return { daemon, dir, session };
  }
  const questions = (daemon: TestDaemon, session: string): PermissionRequest[] =>
    daemon.store
      .read(session)
      .map((r) => r.event)
      .filter((e): e is PermissionRequest => e.type === "permission_request");
  async function until(check: () => boolean): Promise<void> {
    for (let i = 0; i < 500; i++) {
      if (check()) return;
      await Bun.sleep(10);
    }
    throw new Error("timed out");
  }
  const answer = (
    daemon: TestDaemon,
    session: string,
    q: PermissionRequest,
    option: string,
  ): Promise<unknown> =>
    daemon.actions.run(
      { session, ref: "x", action: "neo.permit", args: { id: q.id, option } },
      "test",
    );

  test("its first turn asks, and the answer starts the project's tools service", async () => {
    const { daemon, dir, session } = await opened();
    const started: string[][] = [];
    daemon.projectTools.onChange(() => started.push(...daemon.projectTools.commands()));

    daemon.sessions.input(session, "cli", "hello");
    await until(() => questions(daemon, session).length === 1);
    const asked = questions(daemon, session)[0];
    if (!asked) throw new Error("no question");
    expect(asked.title).toBe("Run this project's tools?");
    expect(asked.details).toEqual([
      { key: "Project", value: dir },
      { key: "Files", value: "progress.ts" },
    ]);
    expect(daemon.projectTools.commands()).toEqual([]);

    await answer(daemon, session, asked, "allow");
    await daemon.sessions.idle(session);
    expect(started).toHaveLength(1);
    expect(started[0]?.slice(3, 7)).toEqual([
      "--dir",
      join(dir, ".neo", "tools"),
      "--app",
      appOf(dir),
    ]);

    // The next turn doesn't ask again.
    daemon.sessions.input(session, "cli", "again");
    await daemon.sessions.idle(session);
    expect(questions(daemon, session)).toHaveLength(1);
  });

  test("a file written during the session asks as soon as the tool call that wrote it ends", async () => {
    const { daemon, dir, session } = await opened();
    daemon.projectTools.decide(dir, fingerprint(join(dir, ".neo", "tools")), "allow");

    // Nothing new: a tool call ending asks nothing.
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t0",
      status: "completed",
    });
    expect(questions(daemon, session)).toHaveLength(0);

    writeTool(dir, "second.ts", "// written by the agent");
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t1",
      status: "completed",
    });
    await until(() => questions(daemon, session).length === 1);
    expect(questions(daemon, session)[0]?.details).toContainEqual({
      key: "Files",
      value: "second.ts",
    });

    // And the tool call that is still running asks nothing.
    writeTool(dir, "third.ts", "// not done yet");
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t2",
      status: "in_progress",
    });
    await Bun.sleep(30);
    expect(questions(daemon, session)).toHaveLength(1);
  });

  test("the setting turns it off: no question, no service", async () => {
    const { daemon, dir, session } = await opened();
    daemon.projectTools.decide(dir, fingerprint(join(dir, ".neo", "tools")), "allow");
    expect(daemon.projectTools.commands()).toHaveLength(1);
    expect(daemon.settings.projectTools()).toBe(true);
    await daemon.commands.run(session, "/settings project-tools off");
    expect(daemon.settings.projectTools()).toBe(false);
    await daemon.commands.run(session, "/settings project-tools on");
    expect(daemon.settings.projectTools()).toBe(true);
    await daemon.commands.run(session, "/settings project-tools off");
    daemon.sessions.input(session, "cli", "hello");
    await daemon.sessions.idle(session);
    expect(questions(daemon, session)).toHaveLength(0);
    expect(daemon.projectTools.commands()).toEqual([]);
  });
});

describe("in the gateway", () => {
  test("a project's tools are found and called from its sessions, and unknown elsewhere", async () => {
    const daemon: TestDaemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const dir = project();
    const elsewhere = project({});
    const inside = daemon.sessions.create({ cwd: dir }).id;
    const outside = daemon.sessions.create({ cwd: elsewhere }).id;
    daemon.projectTools.decide(dir, fingerprint(join(dir, ".neo", "tools")), "allow");

    const app: McpApp = startMcpApp({
      tools: [
        {
          name: "ping",
          description: "Pings the project's server",
          inputSchema: { type: "object", properties: {} },
          handler: () => ({ content: [{ type: "text", text: "pong" }] }),
        },
      ],
    });
    cleanups.push(() => app.close());
    await daemon.connectors.presence(
      {},
      { app: appOf(dir), kind: "service", open: true, mcp: app.endpoint },
    );
    const name = `${appOf(dir)}.ping`;
    const text = (r: Awaited<ReturnType<typeof daemon.gateway.call>>): string => {
      const lines: string[] = [];
      for (const c of r.content) if (c.type === "text") lines.push(c.text);
      return lines.join("\n");
    };

    const found = await daemon.gateway.call("tools_search", { query: "ping" }, undefined, inside);
    expect(text(found)).toContain(name);
    const called = await daemon.gateway.call("tools_call", { name }, undefined, inside);
    expect(text(called)).toBe("pong");

    const missed = await daemon.gateway.call("tools_search", { query: "ping" }, undefined, outside);
    expect(text(missed)).not.toContain(name);
    const refused = await daemon.gateway.call("tools_call", { name }, undefined, outside);
    expect(refused.isError).toBe(true);
    expect(text(refused)).toContain("Unknown tool");
    // The list of known connectors doesn't give the project's away.
    expect(text(refused)).toContain("Known connectors: none");

    // The sessions are told about it, or not, the same way.
    const told = (session: string): string[] =>
      daemon.connectors.snapshot((a) => daemon.projectTools.visible(a, session)).map((c) => c.app);
    expect(told(inside)).toContain(appOf(dir));
    expect(told(outside)).not.toContain(appOf(dir));
  });
});

import { afterEach, describe, expect, test } from "bun:test";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { fingerprint } from "../../services/tools/src/approval.ts";
import { silentLogger } from "../src/log.ts";
import { appOf, projectOf, sessionAppOf, ToolFolders } from "../src/tool-folders.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { startMcpApp, type McpApp } from "./mcp-app.ts";

type PermissionRequest = Extract<SessionEvent, { type: "permission_request" }>;

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function tempDir(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "tool-folders-")));
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

const folderOf = (dir: string): string => join(dir, ".neo", "tools");

/** A project whose .neo/tools holds these files (name → content); none: no folder at all. */
function project(files: Record<string, string> = { "a.ts": "export default [];" }): string {
  const dir = tempDir();
  if (Object.keys(files).length > 0) mkdirSync(folderOf(dir), { recursive: true });
  for (const [name, content] of Object.entries(files)) write(folderOf(dir), name, content);
  return dir;
}

function write(folder: string, name: string, content: string): void {
  const path = join(folder, name);
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

interface Setup {
  make: () => ToolFolders;
  sessions: Map<string, { cwd: string; parent?: string }>;
  asked: { session: string; title: string; details: string }[];
  on: { value: boolean };
  user: string;
}

/** ToolFolders on a fake store, whose questions are answered from `answers`. */
function setup(answers: (string | undefined)[], on = { value: true }): Setup {
  const meta = new Map<string, string>();
  const sessions = new Map<string, { cwd: string; parent?: string }>();
  const asked: Setup["asked"] = [];
  const next = [...answers];
  const dataDir = tempDir();
  const user = join(tempDir(), "tools");
  const make = (): ToolFolders =>
    new ToolFolders({
      store: {
        getMeta: (key: string) => meta.get(key),
        setMeta: (key: string, value: string) => void meta.set(key, value),
        getSession: (id: string) => sessions.get(id),
      },
      asker: {
        ask: (session, question) => {
          if (question.type !== "permission_request") throw new Error("not a permission");
          const files = question.details?.find((d) => d.key === "Files");
          asked.push({ session, title: question.title, details: files?.value ?? "" });
          return Promise.resolve(next.shift());
        },
      },
      agent: "neo",
      log: silentLogger,
      dataDir,
      userFolder: user,
      enabled: () => on.value,
    });
  return { make, sessions, asked, on, user };
}
const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 15));
const allowedFile = (tools: ToolFolders, app: string): Record<string, string> => {
  const stored: Record<string, string> = JSON.parse(readFileSync(tools.allowedPath(app), "utf8"));
  return stored;
};
/** The instances beyond the user's own, which always runs. */
const extra = (tools: ToolFolders): string[][] => tools.commands().slice(1);
const flag = (command: string[] | undefined, name: string): string | undefined =>
  command?.[command.indexOf(`--${name}`) + 1];

describe("allowing a project's files", () => {
  test("the first session asks about the files; yes starts the service and says what it may run", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    const dir = project({ "a.ts": "// a", "b.ts": "// b" });
    sessions.set("s1", { cwd: dir });
    sessions.set("s2", { cwd: dir });
    const tools = make();
    let changes = 0;
    tools.onChange(() => changes++);
    expect(extra(tools)).toEqual([]);

    tools.check("s1");
    tools.check("s2"); // asked already, still waiting
    await settle();
    expect(asked).toEqual([
      { session: "s1", title: "Run this project's tools?", details: "a.ts, b.ts" },
    ]);
    expect(changes).toBe(1);
    const [command] = extra(tools);
    expect(flag(command, "dir")).toBe(folderOf(dir));
    expect(flag(command, "app")).toBe(appOf(dir));
    expect(flag(command, "scope")).toBe("project");
    // What the service holds to: each file by its hash.
    expect(allowedFile(tools, appOf(dir))).toEqual(fingerprint(folderOf(dir)));

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

    write(folderOf(dir), "b.ts", "// b");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "b.ts"]);

    write(folderOf(dir), "a.ts", "// a, changed");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "b.ts", "a.ts (changed)"]);
    expect(allowedFile(tools, appOf(dir))).toEqual(fingerprint(folderOf(dir)));
  });

  test("a helper a tool imports is part of it: changing one asks about every tool file", async () => {
    const { make, sessions, asked } = setup(["allow", "allow"]);
    const dir = project({ "a.ts": "// a", "b.ts": "// b", "lib/say.ts": "export const s = 1;" });
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts, b.ts"]);

    write(folderOf(dir), "lib/say.ts", "export const s = 2;");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts, b.ts", "a.ts (changed), b.ts (changed)"]);
  });

  test("content the user allowed in a write's diff runs without asking; anything else asks", async () => {
    const { make, sessions, asked } = setup(["allow", "allow"]);
    const dir = project({ "a.ts": "// a", "lib/say.ts": "export const s = 1;" });
    const folder = folderOf(dir);
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts"]);

    // Seen whole and allowed: a new tool file, and a helper that changes every tool's hash.
    tools.vouch(join(folder, "b.ts"), "// b");
    write(folder, "b.ts", "// b");
    tools.vouch(join(folder, "lib", "say.ts"), "export const s = 2;");
    write(folder, "lib/say.ts", "export const s = 2;");
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(1);
    expect(allowedFile(tools, appOf(dir))).toEqual(fingerprint(folder));

    // Written with other content than the diff showed: asks.
    tools.vouch(join(folder, "c.ts"), "// c");
    write(folder, "c.ts", "// c, then changed");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "c.ts"]);

    // A helper changed some other way: every tool asks again.
    write(folder, "lib/say.ts", "export const s = 3;");
    tools.vouch(join(folder, "a.ts"), "// a");
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(3);
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
    rmSync(first.allowedPath(appOf(dir)));

    const second = make();
    expect(extra(second)).toHaveLength(1);
    expect(allowedFile(second, appOf(dir))).toEqual(fingerprint(folderOf(dir)));
    second.check("s");
    await settle();
    expect(asked).toHaveLength(1);
  });

  test("no is kept for that content only", async () => {
    const { make, sessions, asked } = setup(["deny", "deny"]);
    const dir = project();
    sessions.set("s", { cwd: dir });
    const tools = make();
    tools.check("s");
    await settle();
    tools.check("s");
    await settle();
    expect(asked).toHaveLength(1);
    expect(extra(tools)).toEqual([]);

    write(folderOf(dir), "a.ts", "// something else");
    tools.check("s");
    await settle();
    expect(asked.map((a) => a.details)).toEqual(["a.ts", "a.ts (changed)"]);
  });

  test("a question dropped unanswered is not put again at once, but on the next check", async () => {
    // A subagent's turn ends with its questions dropped: asked again right away, the question
    // would sit in a session nobody looks at, and hold the folder so no session asks.
    const { make, sessions, asked } = setup([undefined, "allow"]);
    const dir = project();
    sessions.set("child", { cwd: dir, parent: "main" });
    sessions.set("main", { cwd: dir });
    const tools = make();
    tools.check("child");
    await settle();
    expect(asked).toHaveLength(1);

    tools.check("main");
    await settle();
    expect(asked.map((a) => a.session)).toEqual(["child", "main"]);
    expect(extra(tools)).toHaveLength(1);
  });

  test("with the setting off a project's files don't ask or run; on, they ask", async () => {
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
    expect(extra(tools)).toHaveLength(1);
    on.value = false;
    expect(extra(tools)).toEqual([]);
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
    expect(extra(tools)).toEqual([]);
  });

  test("a project's tools are seen from inside it only", () => {
    const { make, sessions } = setup([]);
    const dir = project();
    const other = project();
    sessions.set("in", { cwd: dir });
    sessions.set("deep", { cwd: join(dir, "src") });
    sessions.set("out", { cwd: other });
    const tools = make();
    tools.decide(folderOf(dir), { "a.ts": "hash" }, "allow");
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

describe("the user's own folder", () => {
  test("its instance always runs, and runs only the files the user allowed", async () => {
    const { make, sessions, asked, user } = setup(["allow"]);
    sessions.set("s", { cwd: project({}) });
    const tools = make();
    const [own] = tools.commands();
    expect(flag(own, "app")).toBe("tools");
    expect(flag(own, "scope")).toBe("user");
    expect(flag(own, "approved")).toBe(tools.allowedPath("tools"));
    expect(allowedFile(tools, "tools")).toEqual({});

    // Written with bash, or with approvals on auto: nobody was asked yet.
    write(user, "mine.ts", "// mine");
    tools.check("s");
    await settle();
    expect(asked).toEqual([{ session: "s", title: "Run your tools?", details: "mine.ts" }]);
    expect(allowedFile(tools, "tools")).toEqual(fingerprint(user));
  });
});

describe("a session's own folder", () => {
  test("its files ask, run for that session and its subagents only, and go with it", async () => {
    const { make, sessions, asked } = setup(["allow"]);
    const cwd = project({});
    sessions.set("abcd1234-5678", { cwd });
    sessions.set("sub", { cwd, parent: "abcd1234-5678" });
    sessions.set("other", { cwd });
    const tools = make();
    const folder = tools.sessionFolder("abcd1234-5678");
    write(folder, "deploy.ts", "// one deploy");

    tools.check("other");
    await settle();
    expect(asked).toEqual([]);
    tools.check("abcd1234-5678");
    await settle();
    expect(asked).toEqual([
      { session: "abcd1234-5678", title: "Run this session's tools?", details: "deploy.ts" },
    ]);

    const app = sessionAppOf("abcd1234-5678");
    expect(app).toBe("tools-session-abcd1234");
    const [command] = extra(tools);
    expect(flag(command, "dir")).toBe(folder);
    expect(flag(command, "scope")).toBe("session");
    expect(tools.visible(app, "abcd1234-5678")).toBe(true);
    expect(tools.visible(app, "sub")).toBe(true);
    expect(tools.visible(app, "other")).toBe(false);

    let changes = 0;
    tools.onChange(() => changes++);
    sessions.delete("abcd1234-5678");
    tools.forget("abcd1234-5678");
    expect(changes).toBe(1);
    expect(existsSync(folder)).toBe(false);
    expect(extra(tools)).toEqual([]);
  });

  test("the folder of a session deleted while the daemon was down is removed at start", () => {
    const { make, sessions } = setup([]);
    sessions.set("kept", { cwd: "/" });
    const first = make();
    write(first.sessionFolder("kept"), "a.ts", "// a");
    write(first.sessionFolder("gone"), "a.ts", "// a");
    const second = make();
    expect(existsSync(second.sessionFolder("kept"))).toBe(true);
    expect(existsSync(second.sessionFolder("gone"))).toBe(false);
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
    daemon.toolFolders.onChange(() => started.push(...daemon.toolFolders.commands().slice(1)));

    daemon.sessions.input(session, "cli", "hello");
    await until(() => questions(daemon, session).length === 1);
    const asked = questions(daemon, session)[0];
    if (!asked) throw new Error("no question");
    expect(asked.title).toBe("Run this project's tools?");
    expect(asked.details).toEqual([
      { key: "Project", value: dir },
      { key: "Files", value: "progress.ts" },
    ]);
    expect(daemon.toolFolders.commands().slice(1)).toEqual([]);

    await answer(daemon, session, asked, "allow");
    await daemon.sessions.idle(session);
    expect(started).toHaveLength(1);
    expect(flag(started[0], "app")).toBe(appOf(dir));

    // The next turn doesn't ask again.
    daemon.sessions.input(session, "cli", "again");
    await daemon.sessions.idle(session);
    expect(questions(daemon, session)).toHaveLength(1);
  });

  test("a file written during the session asks as soon as the tool call that wrote it ends", async () => {
    const { daemon, dir, session } = await opened();
    daemon.toolFolders.decide(folderOf(dir), fingerprint(folderOf(dir)), "allow");

    // Nothing new: a tool call ending asks nothing.
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t0",
      status: "completed",
    });
    expect(questions(daemon, session)).toHaveLength(0);

    write(folderOf(dir), "second.ts", "// written by the agent");
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
    write(folderOf(dir), "third.ts", "// not done yet");
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t2",
      status: "in_progress",
    });
    await Bun.sleep(30);
    expect(questions(daemon, session)).toHaveLength(1);
  });

  test("a write the user allowed with the file in its diff is not asked about again", async () => {
    const { daemon, dir, session } = await opened();
    const folder = folderOf(dir);
    daemon.toolFolders.decide(folder, fingerprint(folder), "allow");
    const writeAsked = (id: string, name: string, text: string, option: string): void => {
      daemon.store.append(session, {
        type: "permission_request",
        id,
        agent: "neo",
        title: `Write ${name}?`,
        kind: "edit",
        content: [{ type: "diff", path: join(folder, name), newText: text }],
        options: [
          { id: "allow", name: "Allow", kind: "allow_once" },
          { id: "reject", name: "Reject", kind: "reject_once" },
        ],
      });
      daemon.store.append(session, { type: "permission_resolved", id, option });
      if (option === "allow") write(folder, name, text);
      daemon.store.append(session, {
        type: "tool_call_update",
        toolCallId: id,
        status: "completed",
      });
    };

    writeAsked("w1", "second.ts", "// second", "allow");
    await Bun.sleep(30);
    expect(questions(daemon, session).map((q) => q.title)).toEqual(["Write second.ts?"]);
    expect(daemon.toolFolders.commands().slice(1)).toHaveLength(1);

    // Rejected, then written anyway (by a command): the tool asks.
    writeAsked("w2", "third.ts", "// third", "reject");
    write(folder, "third.ts", "// third");
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "x",
      status: "completed",
    });
    await until(() => questions(daemon, session).length === 3);
    expect(questions(daemon, session)[2]?.details).toContainEqual({
      key: "Files",
      value: "third.ts",
    });
  });

  test("the setting turns it off: no question, no service", async () => {
    const { daemon, dir, session } = await opened();
    daemon.toolFolders.decide(folderOf(dir), fingerprint(folderOf(dir)), "allow");
    expect(daemon.toolFolders.commands().slice(1)).toHaveLength(1);
    await daemon.commands.run(session, "/settings project-tools off");
    expect(daemon.settings.projectTools()).toBe(false);
    daemon.sessions.input(session, "cli", "hello");
    await daemon.sessions.idle(session);
    expect(questions(daemon, session)).toHaveLength(0);
    expect(daemon.toolFolders.commands().slice(1)).toEqual([]);
  });

  test("a deleted session takes its tools folder with it", async () => {
    const { daemon, session } = await opened();
    const folder = daemon.toolFolders.sessionFolder(session);
    write(folder, "once.ts", "// once");
    daemon.sessions.remove(session);
    expect(existsSync(folder)).toBe(false);
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
    daemon.toolFolders.decide(folderOf(dir), fingerprint(folderOf(dir)), "allow");

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
      daemon.connectors.snapshot((a) => daemon.toolFolders.visible(a, session)).map((c) => c.app);
    expect(told(inside)).toContain(appOf(dir));
    expect(told(outside)).not.toContain(appOf(dir));
  });
});

// Delegation as the user sees it: the suite's own MCP tools are not asked about twice, one clear
// "Start a subagent?" gate, a model the harness doesn't list is refused before asking, the
// session's view shows one item per delegation (the subagent's events stay in the log and in its
// own view), and a delegation into a directory or worktree runs as a session there.
import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { createNeoAgent } from "@suite/harness/neo";
import type { Envelope, SessionEvent, ViewItem, ViewPatch } from "@suite/protocol/types";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { fakeModel, reply, TEST_CONFIG } from "../../harness/test/fake-model.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

async function setup(
  options: { gate?: string; cwd?: string } = {},
): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({ agent: new FakeAgent("main"), guests: { fake: FAKE } });
  cleanups.push(() => daemon.stop());
  const gate = options.gate ?? "allow";
  if (gate !== "manual") {
    daemon.store.onAppend(({ session, event }) => {
      if (event.type !== "permission_request" || event.title !== "Start a subagent?") return;
      void Promise.resolve().then(() =>
        daemon.actions.run(
          { session, ref: "p", action: "suite.permit", args: { id: event.id, option: gate } },
          "test",
        ),
      );
    });
  }
  const session = daemon.sessions.create({ cwd: options.cwd }).id;
  return { daemon, session };
}

function git(dir: string, ...args: string[]): string {
  const run = Bun.spawnSync(["git", "-C", dir, ...args], { env: { ...process.env } });
  if (run.exitCode !== 0) throw new Error(`git ${args.join(" ")}: ${run.stderr.toString()}`);
  return run.stdout.toString().trim();
}

/** A repository with one commit in a temp dir. */
function gitRepo(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "suite-repo-")));
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
  git(dir, "init", "-q", "-b", "main");
  git(
    dir,
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@example.com",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "init",
  );
  return dir;
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

function resultOf(result: CallToolResult): string {
  return (result.content[0] as { text: string }).text;
}

function delegate(
  daemon: TestDaemon,
  session: string,
  args: Record<string, string>,
): Promise<CallToolResult> {
  return daemon.gateway.call("delegate", { agent: "fake", task: "x", ...args }, undefined, session);
}

/** Answers the oldest open question of the session. */
async function answer(
  daemon: TestDaemon,
  session: string,
  option: string,
  type: "permission_request" | "choice" = "permission_request",
): Promise<Extract<SessionEvent, { type: "permission_request" }>> {
  const open = (): Extract<SessionEvent, { type: "permission_request" }> | undefined => {
    const log = events(daemon, session);
    return log.find(
      (e): e is Extract<SessionEvent, { type: "permission_request" }> =>
        e.type === type &&
        !log.some((r) => r.type === "permission_resolved" && "id" in e && r.id === e.id),
    );
  };
  await until(() => open() !== undefined, type);
  const request = open();
  if (!request) throw new Error("no open question");
  await daemon.actions.run(
    { session, ref: "x", action: "suite.permit", args: { id: request.id, option } },
    "test",
  );
  return request;
}

/** The user's turn "/fake <task>"; resolves with the guest's reply once the turn is over. */
async function guestTurn(daemon: TestDaemon, session: string, task: string): Promise<string> {
  daemon.sessions.input(session, "cli", `/fake ${task}`);
  await daemon.sessions.idle(session);
  const turn = events(daemon, session).findLast((e) => e.type === "guest_turn");
  if (turn?.type !== "guest_turn") throw new Error("no guest_turn");
  return turn.result;
}

describe("the suite's own MCP tools", () => {
  test("are allowed without a question in every mode, whatever the adapter calls them", async () => {
    const { daemon, session } = await setup();
    for (const mode of ["ask", "review", "edits", "auto"]) {
      await daemon.commands.run(session, `/approvals ${mode}`);
      for (const tool of ["tools_search", "tools_call", "apps_launch", "delegate"]) {
        expect(await guestTurn(daemon, session, `suitetool ${tool}`)).toBe("answer: yes");
      }
      // Codex's spelling.
      expect(await guestTurn(daemon, session, "codex tools_call")).toBe("answer: yes");
    }
    expect(events(daemon, session).some((e) => e.type === "permission_request")).toBe(false);
  });

  test("another tool of the same adapter still asks", async () => {
    const { daemon, session } = await setup();
    daemon.sessions.input(session, "cli", "/fake suitetool nothing_of_ours");
    const request = await answer(daemon, session, "yes");
    // Titled by what it does, like any request.
    expect(request.title).toBe("nothing_of_ours");
    await daemon.sessions.idle(session);
  });
});

describe("the gate", () => {
  test("a compact question with the harness, model, effort and directory, and the task", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    const task = Array.from({ length: 30 }, (_, i) => `step ${i + 1}`).join("\n");
    const call = delegate(daemon, session, { task, model: "large", effort: "high" });
    const request = await answer(daemon, session, "allow");
    expect(request).toMatchObject({
      agent: "main",
      title: "Start a subagent?",
      details: [
        { key: "Harness", value: "Fake" },
        { key: "Model", value: "large" },
        { key: "Effort", value: "high" },
        { key: "Directory", value: daemon.sessions.info(session)?.cwd },
      ],
      content: [{ type: "text", text: task }],
    });
    await call;

    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${request.id}`);
    // Answered: the facts stay, the task and buttons are gone.
    expect(item).toMatchObject({ label: "Start a subagent?", state: "done" });
    if (item?.component.type !== "stack") throw new Error("not a stack");
    const [kv, outcome] = item.component.children;
    expect(
      kv?.type === "kv" && kv.items.map((entry) => [entry.key, entry.value, entry.tone]),
    ).toEqual([
      ["Harness", "Fake", "accent"],
      ["Model", "large", "accent"],
      ["Effort", "high", "accent"],
      ["Directory", daemon.sessions.info(session)?.cwd, "accent"],
    ]);
    expect(JSON.stringify(outcome)).toContain("Start Fake");
    expect(JSON.stringify(item)).not.toContain("step 1");
  });

  test("while it waits: highlighted values, the task as code, one button per answer", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    const call = delegate(daemon, session, { task: "line 1\nline 2" });
    await until(() => events(daemon, session).some((e) => e.type === "permission_request"), "ask");
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref.startsWith("p-"));
    if (item?.component.type !== "stack") throw new Error("not a stack");
    const [kv, code, row] = item.component.children;
    expect(kv).toMatchObject({ type: "kv" });
    expect(kv?.type === "kv" && kv.items.every((entry) => entry.tone === "accent")).toBe(true);
    expect(code).toEqual({ type: "code", code: "line 1\nline 2" });
    expect(row?.type === "row" && row.children.map((b) => b.type === "button" && b.label)).toEqual([
      "Start Fake",
      "Allow Fake for this session",
      "Reject",
    ]);
    await answer(daemon, session, "reject");
    await call;
  });
});

describe("the models tool", () => {
  test("starts a harness that isn't running to list what it takes, by delegate's names", async () => {
    const { daemon, session } = await setup();
    const all = await daemon.gateway.call("models", {}, undefined, session);
    expect(all.isError).toBeUndefined();
    const listed = resultOf(all);
    expect(listed).toContain("fake (Fake)\n  models:\n    small");
    expect(listed).toContain("    large");
    expect(listed).toContain("  efforts:\n    low");
    expect(listed).toContain("Give delegate a value as its model or effort.");

    const narrowed = resultOf(
      await daemon.gateway.call("models", { agent: "fake", query: "lar" }, undefined, session),
    );
    expect(narrowed).toContain("    large");
    expect(narrowed).not.toContain("    small");

    const unknown = await daemon.gateway.call("models", { agent: "nope" }, undefined, session);
    expect(unknown.isError).toBe(true);
    expect(resultOf(unknown)).toStartWith("No guest named nope");
  });
});

describe("a model or effort the harness lists and doesn't know", () => {
  test("is refused with the valid names before anything is asked", async () => {
    const { daemon, session } = await setup();
    // The adapter has to be running to list its models.
    await delegate(daemon, session, { task: "config" });
    const asked = events(daemon, session).filter((e) => e.type === "permission_request").length;

    const huge = await delegate(daemon, session, { model: "huge" });
    expect(huge.isError).toBe(true);
    expect(resultOf(huge)).toBe('Unknown model "huge" for Fake. Valid models: small, large.');
    const odd = await delegate(daemon, session, { effort: "extreme" });
    expect(resultOf(odd)).toBe('Unknown effort "extreme" for Fake. Valid efforts: low, high.');
    expect(events(daemon, session).filter((e) => e.type === "permission_request")).toHaveLength(
      asked,
    );

    // Case and the display name don't matter; the harness gets its own spelling.
    const set = await delegate(daemon, session, { task: "config", model: "LARGE", effort: "High" });
    expect(JSON.parse(resultOf(set).split("\n\n").at(-1) ?? "")).toMatchObject({
      config: { model: "large", effort: "high" },
    });
  });

  test("an adapter that isn't running yet isn't started just to list its models", async () => {
    const { daemon, session } = await setup({ gate: "manual" });
    const call = delegate(daemon, session, { model: "huge" });
    const request = await answer(daemon, session, "allow");
    expect(request.details).toContainEqual({ key: "Model", value: "huge" });
    // The adapter's own complaint comes after the user agreed.
    expect(resultOf(await call)).toContain("no model huge");
  });
});

describe("the session's view of a delegation", () => {
  /** The calls a main harness makes around the gateway's delegate (here: by hand). */
  async function mainDelegates(
    daemon: TestDaemon,
    session: string,
    task: string,
  ): Promise<CallToolResult> {
    const rawInput = { agent: "fake", task };
    daemon.store.append(session, {
      type: "tool_call",
      toolCallId: "m1",
      tool: "delegate",
      title: "fake · x",
      kind: "think",
      status: "pending",
      rawInput,
    });
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "m1",
      status: "in_progress",
    });
    const result = await daemon.gateway.call("delegate", rawInput, undefined, session);
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "m1",
      status: "completed",
      content: [{ type: "text", text: resultOf(result) }],
    });
    return result;
  }

  test("one tool item with the subagent's reply; its own work stays in the log", async () => {
    const { daemon, session } = await setup();
    await mainDelegates(daemon, session, "work on the greeting\ngreet Ada");
    const log = events(daemon, session);
    const at = (type: string): number => log.findIndex((e) => e.type === type);
    expect(at("subagent_started")).toBeGreaterThan(at("tool_call"));
    expect(log.find((e) => e.type === "subagent_started")).toMatchObject({
      agent: "fake",
      task: "work on the greeting\ngreet Ada",
    });
    expect(log.find((e) => e.type === "subagent_ended")).toMatchObject({ stopReason: "end_turn" });
    // Still there for whoever wants to look.
    expect(log.some((e) => e.type === "agent_thought_chunk" && e.agent === "fake")).toBe(true);
    expect(log.some((e) => e.type === "tool_call" && e.agent === "fake")).toBe(true);

    const items = daemon.views.get(session).snapshot().items;
    expect(items.map((i) => i.ref.split("-")[0])).toEqual(["c", "p"]);
    const call = items.find((i) => i.ref === "c-m1");
    expect(call).toMatchObject({
      kind: "tool",
      label: "fake · work on the greeting",
      state: "done",
    });
    expect(JSON.stringify(call?.component)).toContain("Done: greet Ada");
  });

  test("a running delegation reads running, and a failed one failed", async () => {
    const { daemon, session } = await setup();
    daemon.store.append(session, {
      type: "tool_call",
      toolCallId: "m2",
      tool: "delegate",
      title: "delegate",
      kind: "think",
      status: "in_progress",
      rawInput: { agent: "claude", task: "refactor" },
    });
    // A harness that titles the call "delegate" (Claude Code) still gets the label.
    expect(daemon.views.get(session).snapshot().items[0]).toMatchObject({
      label: "claude · refactor",
      state: "running",
    });
    daemon.store.append(session, { type: "tool_call_update", toolCallId: "m2", status: "failed" });
    expect(daemon.views.get(session).snapshot().items[0]).toMatchObject({ state: "failed" });
  });

  test("a guest the user addressed directly stays inline", async () => {
    const { daemon, session } = await setup();
    await guestTurn(daemon, session, "hello");
    const items = daemon.views.get(session).snapshot().items;
    expect(items.some((i) => i.kind === "agent" && i.label === "fake")).toBe(true);
  });

  test("a subagent that never ended doesn't hide the next turn", async () => {
    const { daemon, session } = await setup();
    daemon.store.append(session, { type: "subagent_started", id: "d1", agent: "fake", task: "x" });
    daemon.store.append(session, {
      type: "agent_message_chunk",
      messageId: "fake-1",
      text: "hidden",
      agent: "fake",
    });
    expect(daemon.views.get(session).snapshot().items).toEqual([]);
    daemon.store.append(session, { type: "turn_ended", stopReason: "error", error: "crash" });
    daemon.store.append(session, {
      type: "agent_message_chunk",
      messageId: "fake-2",
      text: "shown",
      agent: "fake",
    });
    expect(
      daemon.views
        .get(session)
        .snapshot()
        .items.map((i) => i.label),
    ).toEqual([undefined, "fake"]);
  });
});

describe("one agent's transcript", () => {
  test("a view of just that agent's work, snapshot and patches, over the bus", async () => {
    const { daemon, session } = await setup();
    const received: Envelope[] = [];
    const handle = daemon.hub.connect({ send: (frame) => received.push(JSON.parse(frame)) });
    const send = (envelope: Record<string, unknown>): void =>
      handle.receive(JSON.stringify(envelope));
    send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
    send({ topic: "view.subscribe", id: "main", payload: { session } });
    send({ topic: "view.subscribe", id: "fake", payload: { session, agent: "fake" } });
    const snapshots = received.filter((e) => e.topic === "view.snapshot");
    expect(snapshots.map((e) => e.topic === "view.snapshot" && e.payload.agent)).toEqual([
      undefined,
      "fake",
    ]);

    daemon.store.append(session, { type: "user_message", app: "cli", text: "hello main" });
    await delegate(daemon, session, { task: "greet Ada" });
    const patches = received.filter(
      (e): e is Extract<Envelope, { topic: "view.patch" }> => e.topic === "view.patch",
    );
    const own = patches.filter((e) => e.payload.agent === "fake");
    const main = patches.filter((e) => e.payload.agent === undefined);
    expect(own.length).toBeGreaterThan(0);
    // The main view saw its own conversation and the gate, nothing of the subagent's work.
    const refs = (list: typeof patches): string[] => {
      const out: string[] = [];
      for (const e of list) for (const op of e.payload.ops) if ("item" in op) out.push(op.item.ref);
      return out;
    };
    expect(refs(main).some((r) => r.startsWith("c-fake:") || r.startsWith("t-fake"))).toBe(false);

    const view = daemon.views.get(session, "fake").snapshot();
    expect(view.agent).toBe("fake");
    const kinds = view.items.map((i: ViewItem) => `${i.kind}:${i.label ?? ""}`);
    // The task, its thought and replies, the call to greeter.greet; no user message of the main chat.
    expect(kinds[0]).toBe("user:task");
    expect(kinds).toContain("thought:");
    expect(kinds).toContain("tool:greeter.greet");
    expect(view.items.some((i) => i.kind === "user" && i.label === "cli")).toBe(false);
    expect(view.items.some((i) => i.ref.startsWith("p-"))).toBe(false);

    // Re-subscribing without the agent still gives the session's own view.
    send({ topic: "view.unsubscribe", id: "u", payload: { session, agent: "fake" } });
    const before = received.length;
    await delegate(daemon, session, { task: "again" });
    const later: ViewPatch[] = [];
    for (const e of received.slice(before)) if (e.topic === "view.patch") later.push(e.payload);
    expect(later.some((p) => p.agent === "fake")).toBe(false);
  });
});

describe("delegating into a directory", () => {
  test("a worktree is created and the subagent runs as a session there, linked to the parent", async () => {
    const repo = gitRepo();
    const { daemon, session } = await setup({ gate: "manual", cwd: repo });
    const call = delegate(daemon, session, { task: "cwd", worktree: "fix-login" });
    const request = await answer(daemon, session, "allow");
    const dir = join(repo, ".worktrees", "fix-login");
    expect(request.details).toContainEqual({
      key: "Directory",
      value: `${dir} (new worktree, branch fix-login)`,
    });
    const result = await call;
    expect(result.isError).toBeUndefined();
    expect(resultOf(result)).toContain(`cwd: ${dir}`);

    // A real worktree on its own branch, kept out of git status.
    expect(existsSync(join(dir, ".git"))).toBe(true);
    expect(git(repo, "branch", "--list", "fix-login")).toContain("fix-login");
    expect(git(repo, "status", "--porcelain")).toBe("");

    const started = events(daemon, session).find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started" || started.child === undefined) {
      throw new Error("no child session");
    }
    expect(started).toMatchObject({ agent: "fake", cwd: dir });
    const child = daemon.sessions.info(started.child);
    expect(child).toMatchObject({ parent: session, cwd: dir });
    expect(daemon.sessions.list().find((s) => s.id === session)?.parent).toBeUndefined();
    // The child is a session of the delegated harness; the work happened there, not in the parent.
    expect(daemon.settings.harness(started.child)).toBe("fake");
    const childLog = events(daemon, started.child);
    expect(childLog.find((e) => e.type === "user_message")).toMatchObject({
      app: "delegate",
      text: "cwd",
    });
    expect(childLog.some((e) => e.type === "agent_message_chunk")).toBe(true);
    expect(events(daemon, session).some((e) => e.type === "agent_message_chunk")).toBe(false);
    expect(events(daemon, session).find((e) => e.type === "subagent_ended")).toMatchObject({
      stopReason: "end_turn",
    });

    // The second time the worktree exists and the same session continues.
    const again = delegate(daemon, session, { task: "started", worktree: "fix-login" });
    const second = await answer(daemon, session, "allow");
    expect(second.details).toContainEqual({ key: "Directory", value: `${dir} (worktree)` });
    expect(resultOf(await again)).toContain("started: new");
    const children = daemon.sessions.list().filter((s) => s.parent === session);
    expect(children.map((s) => s.id)).toEqual([started.child]);
    expect(events(daemon, started.child).filter((e) => e.type === "user_message")).toHaveLength(2);
  });

  test("cwd may be relative to the session's; the session's own directory stays in-session", async () => {
    const repo = gitRepo();
    mkdirSync(join(repo, "sub"));
    const { daemon, session } = await setup({ cwd: repo });
    const result = await delegate(daemon, session, { task: "cwd", cwd: "sub" });
    expect(resultOf(result)).toContain(`cwd: ${join(repo, "sub")}`);

    const same = await delegate(daemon, session, { task: "cwd", cwd: "." });
    // Not a child session: it runs in the session itself, whose adapter was started in the repo.
    expect(resultOf(same)).toBe(`cwd: ${repo}`);
    expect(daemon.sessions.list().filter((s) => s.parent === session)).toHaveLength(1);
  });

  test("bad directories are refused before anything is asked", async () => {
    const repo = gitRepo();
    const plain = mkdtempSync(join(tmpdir(), "suite-plain-"));
    cleanups.push(() => rmSync(plain, { recursive: true, force: true }));
    const { daemon, session } = await setup({ gate: "manual", cwd: repo });
    const refused = async (args: Record<string, string>, cwdSession = session): Promise<string> => {
      const result = await delegate(daemon, cwdSession, args);
      expect(result.isError).toBe(true);
      return resultOf(result);
    };
    expect(await refused({ cwd: "a", worktree: "b" })).toContain("cwd or worktree, not both");
    expect(await refused({ worktree: "../escape" })).toContain("not a usable worktree name");
    expect(await refused({ worktree: "a/b" })).toContain("not a usable worktree name");
    expect(await refused({ cwd: "missing" })).toContain("is not a directory");
    const outside = daemon.sessions.create({ cwd: plain }).id;
    expect(await refused({ worktree: "x" }, outside)).toContain("in a git repository");
    expect(existsSync(join(repo, ".worktrees"))).toBe(false);
    for (const id of [session, outside]) {
      expect(events(daemon, id).some((e) => e.type === "permission_request")).toBe(false);
    }
  });

  test("neo in a worktree is a session of the daemon's neo, working there", async () => {
    const repo = gitRepo();
    const model = fakeModel([reply("Hi from the worktree.")]);
    const neo = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const daemon = await startTestDaemon({ agent: neo });
    cleanups.push(() => daemon.stop());
    daemon.store.onAppend(({ session, event }) => {
      if (event.type !== "permission_request" || event.title !== "Start a subagent?") return;
      void Promise.resolve().then(() =>
        daemon.actions.run(
          { session, ref: "p", action: "suite.permit", args: { id: event.id, option: "allow" } },
          "test",
        ),
      );
    });
    const session = daemon.sessions.create({ cwd: repo }).id;
    const result = await daemon.gateway.call(
      "delegate",
      { agent: "neo", task: "say hi", worktree: "neo-w" },
      undefined,
      { session, text: true },
    );
    expect(resultOf(result)).toContain("Hi from the worktree.");
    const dir = join(repo, ".worktrees", "neo-w");
    expect(existsSync(dir)).toBe(true);
    const child = daemon.sessions.list().find((s) => s.parent === session);
    expect(child).toMatchObject({ cwd: dir });
    expect(model.requests).toHaveLength(1);
    // The parent's log has the delegation, not neo's words.
    expect(events(daemon, session).some((e) => e.type === "agent_message_chunk")).toBe(false);
  });

  test("the child's questions are shown, and answered, in the parent", async () => {
    const repo = gitRepo();
    const { daemon, session } = await setup({ cwd: repo });
    const call = delegate(daemon, session, { task: "ask", worktree: "q" });
    // The fake adapter asks to write a file: the card is in the parent's log.
    const request = await answer(daemon, session, "yes");
    expect(request).toMatchObject({ title: "Write notes.txt" });
    expect(resultOf(await call)).toContain("answer: yes");
    const child = daemon.sessions.list().find((s) => s.parent === session)?.id ?? "";
    // The same question and its answer are in the child's log too.
    expect(events(daemon, child).filter((e) => e.type === "permission_resolved")).toHaveLength(1);
    expect(
      daemon.views
        .get(session)
        .snapshot()
        .items.some((i) => i.ref === `p-${request.id}` && i.state === "done"),
    ).toBe(true);
  });

  test("a delegated session can't delegate; its approvals start as the parent's", async () => {
    const repo = gitRepo();
    const { daemon, session } = await setup({ cwd: repo });
    await daemon.commands.run(session, "/approvals edits");
    await delegate(daemon, session, { task: "started", worktree: "w" });
    const child = daemon.sessions.list().find((s) => s.parent === session)?.id ?? "";
    expect(daemon.settings.approvals(child)).toBe("edits");
    const refused = await daemon.gateway.call("delegate", { agent: "fake", task: "x" }, undefined, {
      session: child,
      text: true,
    });
    expect(resultOf(refused)).toBe("Subagents can't delegate.");
  });
});

describe("stopping one subagent", () => {
  test("suite.cancel_agent stops it; the delegate call reports it and the turn goes on", async () => {
    const { daemon, session } = await setup();
    const call = delegate(daemon, session, { task: "wait" });
    await until(() => events(daemon, session).some((e) => e.type === "subagent_started"), "start");
    const started = events(daemon, session).find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started") throw new Error("no start");
    await daemon.actions.run(
      { session, ref: "agents", action: "suite.cancel_agent", args: { id: started.id } },
      "test",
    );
    const result = await call;
    expect(result.isError).toBe(true);
    expect(resultOf(result)).toContain("fake stopped: cancelled");
    expect(events(daemon, session).find((e) => e.type === "subagent_ended")).toMatchObject({
      id: started.id,
      stopReason: "cancelled",
    });
    // Gone: a second stop is an error.
    expect(
      daemon.actions.run(
        { session, ref: "agents", action: "suite.cancel_agent", args: { id: started.id } },
        "test",
      ),
    ).rejects.toThrow("isn't a running subagent");
  });

  test("also one in a worktree", async () => {
    const repo = gitRepo();
    const { daemon, session } = await setup({ cwd: repo });
    const call = delegate(daemon, session, { task: "wait", worktree: "slow" });
    await until(() => events(daemon, session).some((e) => e.type === "subagent_started"), "start");
    const started = events(daemon, session).find((e) => e.type === "subagent_started");
    if (started?.type !== "subagent_started") throw new Error("no start");
    // Let the child's turn begin before stopping it.
    const child = started.child ?? "";
    await until(() => daemon.sessions.info(child)?.state === "running", "the child's turn");
    await Bun.sleep(100);
    await daemon.actions.run(
      { session, ref: "agents", action: "suite.cancel_agent", args: { id: started.id } },
      "test",
    );
    const result = await call;
    expect(result.isError).toBe(true);
    expect(events(daemon, child).findLast((e) => e.type === "turn_ended")).toMatchObject({
      stopReason: "cancelled",
    });
  });
});

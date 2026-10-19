import { afterEach, describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { RequestPermission } from "@suite/harness/agent";
import type { DiffContent, SessionEvent, ViewItem } from "@suite/protocol/types";

type Choice = Extract<SessionEvent, { type: "choice" }>;
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function setup(): Promise<{ daemon: TestDaemon; session: string; agent: FakeAgent }> {
  const agent = new FakeAgent("neo");
  const daemon = await startTestDaemon({ agent });
  cleanups.push(() => daemon.stop());
  return { daemon, session: daemon.sessions.create({}).id, agent };
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

/** Runs a turn so the agent session exists, and returns the hook the daemon gave it. */
async function hookOf(
  daemon: TestDaemon,
  session: string,
  agent: FakeAgent,
): Promise<RequestPermission> {
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(1);
  agent.release();
  await daemon.sessions.idle(session);
  const hook = agent.created[0]?.requestPermission;
  if (!hook) throw new Error("the daemon gave the agent no permission hook");
  return hook;
}

function requests(daemon: TestDaemon, session: string): SessionEvent[] {
  return events(daemon, session).filter((e) => e.type === "permission_request");
}

async function press(
  daemon: TestDaemon,
  session: string,
  id: string,
  option: string,
  note?: string,
): Promise<void> {
  const args: Record<string, string> = { id, option };
  if (note !== undefined) args.note = note;
  await daemon.actions.run({ session, ref: "x", action: "suite.permit", args }, "t");
}

const diff: DiffContent = { type: "diff", path: "/w/a.ts", oldText: "a\n", newText: "b\n" };

describe("the agent's own tools by approvals mode", () => {
  test("auto never asks", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals auto");
    expect(await ask({ title: "rm -rf x", kind: "execute" })).toEqual({ allowed: true });
    expect(await ask({ title: "write a", kind: "edit", content: [diff] })).toEqual({
      allowed: true,
    });
    expect(requests(daemon, session)).toEqual([]);
  });

  test("edits goes ahead with edits and asks for the rest", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals edits");
    expect(await ask({ title: "write a", kind: "edit", content: [diff] })).toEqual({
      allowed: true,
    });
    expect(requests(daemon, session)).toEqual([]);
    const pending = ask({ title: "ls", kind: "execute", toolCallId: "tc1" });
    await until(() => requests(daemon, session).length === 1, "the question");
    const [request] = requests(daemon, session);
    if (request?.type !== "permission_request") throw new Error("no request");
    expect(request).toMatchObject({
      agent: "neo",
      // The command is shown as code, under a plain question.
      title: "Run this command?",
      content: [{ type: "text", text: "ls" }],
      kind: "execute",
      toolCallId: "tc1",
      options: [
        { id: "allow", name: "Allow", kind: "allow_once" },
        { id: "always", name: "Allow for this session", kind: "allow_always" },
        { id: "reject", name: "Reject", kind: "reject_once" },
      ],
    });
    await press(daemon, session, request.id, "reject");
    expect(await pending).toEqual({ allowed: false });
  });

  test("ask asks for edits with the diff; allow-for-session is per kind", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    const pending = ask({ title: "edit a.ts", kind: "edit", toolCallId: "tc", content: [diff] });
    await until(() => requests(daemon, session).length === 1, "the question");
    const [request] = requests(daemon, session);
    if (request?.type !== "permission_request") throw new Error("no request");
    expect(request).toMatchObject({ kind: "edit", content: [diff] });

    // Its card shows the diff above the buttons, then collapses to the answer.
    const item = (): ViewItem | undefined =>
      daemon.views
        .get(session)
        .snapshot()
        .items.find((i) => i.ref === `p-${request.id}`);
    expect(item()).toMatchObject({
      label: "neo asks: edit a.ts",
      state: "pending",
      component: {
        type: "stack",
        children: [{ type: "diff", path: "/w/a.ts" }, { type: "row" }],
      },
    });
    await press(daemon, session, request.id, "always");
    expect(await pending).toEqual({ allowed: true });
    expect(item()).toMatchObject({ state: "done", component: { type: "text" } });

    // Edits are remembered for the session; commands and other sessions still ask.
    expect(await ask({ title: "edit b.ts", kind: "edit", content: [diff] })).toEqual({
      allowed: true,
    });
    expect(requests(daemon, session)).toHaveLength(1);
    const bash = ask({ title: "ls", kind: "execute" });
    await until(() => requests(daemon, session).length === 2, "the second question");
    const second = requests(daemon, session)[1];
    if (second?.type !== "permission_request") throw new Error("no request");
    await press(daemon, session, second.id, "allow");
    expect(await bash).toEqual({ allowed: true });

    // A cancelled turn drops its open question: nobody answered, so no.
    const controller = new AbortController();
    const aborted = ask({ title: "pwd", kind: "execute" }, controller.signal);
    await until(() => requests(daemon, session).length === 3, "the third question");
    controller.abort();
    expect(await aborted).toEqual({ allowed: false });
    const last = events(daemon, session).findLast((e) => e.type === "permission_resolved");
    expect(last).not.toHaveProperty("option");
  });
});

/** The permission_request that is open: waits for the next one after `seen` requests. */
async function nextRequest(
  daemon: TestDaemon,
  session: string,
  seen: number,
): Promise<Extract<SessionEvent, { type: "permission_request" }>> {
  await until(() => requests(daemon, session).length > seen, "the question");
  const request = requests(daemon, session)[seen];
  if (request?.type !== "permission_request") throw new Error("no request");
  return request;
}

describe("reads and sensitive files", () => {
  const read = (path: string): { title: string; kind: "read"; path: string } => ({
    title: `read ${path}`,
    kind: "read",
    path,
  });

  test("an ordinary read never asks, in any mode", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await writeFile(join(daemon.dir, "notes.md"), "hi\n");
    for (const mode of ["ask", "review", "edits", "auto"]) {
      await daemon.commands.run(session, `/approvals ${mode}`);
      expect(await ask(read(join(daemon.dir, "notes.md")))).toEqual({ allowed: true });
      expect(await ask(read(join(daemon.dir, ".env.example")))).toEqual({ allowed: true });
    }
    expect(requests(daemon, session)).toEqual([]);
  });

  test("a sensitive read asks in every mode, with kind read and the file in the title", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    let seen = 0;
    for (const mode of ["ask", "review", "edits", "auto"]) {
      await daemon.commands.run(session, `/approvals ${mode}`);
      const pending = ask({ ...read(join(daemon.dir, ".env")), toolCallId: "tc" });
      const request = await nextRequest(daemon, session, seen++);
      expect(request).toMatchObject({
        agent: "neo",
        title: "Read .env?",
        kind: "read",
        toolCallId: "tc",
        options: [{ id: "allow" }, { id: "always" }, { id: "reject" }],
      });
      await press(daemon, session, request.id, "reject");
      expect(await pending).toEqual({ allowed: false });
    }
  });

  test("allowing for the session is per file", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals auto");
    const env = join(daemon.dir, ".env");
    const first = ask(read(env));
    const request = await nextRequest(daemon, session, 0);
    await press(daemon, session, request.id, "always");
    expect(await first).toEqual({ allowed: true });
    expect(await ask(read(env))).toEqual({ allowed: true });
    // A command on the same file is covered too; another file still asks.
    expect(await ask({ title: "cat .env", kind: "execute" })).toEqual({ allowed: true });
    expect(requests(daemon, session)).toHaveLength(1);
    const other = ask(read(join(daemon.dir, ".env.local")));
    const second = await nextRequest(daemon, session, 1);
    expect(second.title).toBe("Read .env.local?");
    await press(daemon, session, second.id, "allow");
    expect(await other).toEqual({ allowed: true });
    // Nothing was remembered for it.
    const controller = new AbortController();
    const again = ask(read(join(daemon.dir, ".env.local")), controller.signal);
    await nextRequest(daemon, session, 2);
    controller.abort();
    expect(await again).toEqual({ allowed: false });
  });

  test("a command that mentions a sensitive file asks even in auto", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals auto");
    expect(await ask({ title: "ls -la && cat README.md", kind: "execute" })).toEqual({
      allowed: true,
    });
    expect(await ask({ title: "cat .env.example", kind: "execute" })).toEqual({ allowed: true });
    expect(requests(daemon, session)).toEqual([]);
    const pending = ask({ title: "cat .env | head", kind: "execute", toolCallId: "tc" });
    const request = await nextRequest(daemon, session, 0);
    expect(request).toMatchObject({
      title: "This command touches .env",
      kind: "execute",
      toolCallId: "tc",
      content: [{ type: "text", text: "cat .env | head" }],
    });
    await press(daemon, session, request.id, "allow");
    expect(await pending).toEqual({ allowed: true });

    // An allowed sensitive command doesn't ask a second time in ask mode.
    await daemon.commands.run(session, "/approvals ask");
    const home = ask({ title: "grep x ~/.ssh/config", kind: "execute" });
    const second = await nextRequest(daemon, session, 1);
    expect(second.title).toBe("This command touches ~/.ssh/config");
    await press(daemon, session, second.id, "allow");
    expect(await home).toEqual({ allowed: true });
    expect(requests(daemon, session)).toHaveLength(2);
  });
});

describe("review mode, the agent's own tools", () => {
  const edit = { title: "edit a.ts", kind: "edit" as const, toolCallId: "tc", content: [] };

  async function written(
    daemon: TestDaemon,
    name: string,
    before: string | undefined,
    after: string,
  ): Promise<DiffContent> {
    const path = join(daemon.dir, name);
    await writeFile(path, after);
    return { type: "diff", path, oldText: before, newText: after };
  }

  test("edits go ahead and ask to be reviewed once written; other things ask as in ask mode", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals review");
    expect(await ask({ ...edit, content: [diff] })).toEqual({ allowed: true, review: true });
    expect(requests(daemon, session)).toEqual([]);
    const pending = ask({ title: "ls", kind: "execute" });
    const request = await nextRequest(daemon, session, 0);
    expect(request.content).toEqual([{ type: "text", text: "ls" }]);
    await press(daemon, session, request.id, "allow");
    expect(await pending).toEqual({ allowed: true });
  });

  test("keep: the card shows the diff and Keep / Revert; the file stays", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals review");
    const change = await written(daemon, "a.ts", "old\n", "new\n");
    const pending = ask({ ...edit, phase: "after", content: [change] });
    const request = await nextRequest(daemon, session, 0);
    expect(request).toMatchObject({
      agent: "neo",
      title: "Edited a.ts — keep it?",
      kind: "edit",
      toolCallId: "tc",
      content: [change],
      options: [
        { id: "keep", name: "Keep", kind: "allow_once" },
        { id: "revert", name: "Revert", kind: "reject_once" },
      ],
    });
    // The card is a diff above two buttons.
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${request.id}`);
    expect(item).toMatchObject({
      label: "neo asks: Edited a.ts — keep it?",
      state: "pending",
      component: { type: "stack", children: [{ type: "diff" }, { type: "row" }] },
    });
    await press(daemon, session, request.id, "keep");
    expect(await pending).toEqual({ allowed: true });
    expect(await readFile(join(daemon.dir, "a.ts"), "utf8")).toBe("new\n");
  });

  test("revert puts the old content back, deletes a created file, and keeps the note", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals review");
    const change = await written(daemon, "a.ts", "old\n", "new\n");
    const first = ask({ ...edit, phase: "after", content: [change] });
    await press(
      daemon,
      session,
      (await nextRequest(daemon, session, 0)).id,
      "revert",
      "wrong file",
    );
    expect(await first).toEqual({ allowed: false, reverted: true, note: "wrong file" });
    expect(await readFile(join(daemon.dir, "a.ts"), "utf8")).toBe("old\n");

    const created = await written(daemon, "made.ts", undefined, "x\n");
    const second = ask({ ...edit, phase: "after", content: [created] });
    await press(daemon, session, (await nextRequest(daemon, session, 1)).id, "revert");
    expect(await second).toEqual({ allowed: false, reverted: true });
    expect(existsSync(join(daemon.dir, "made.ts"))).toBe(false);
  });

  test("a file that changed in between is left alone, and the answer says so", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals review");
    const change = await written(daemon, "a.ts", "old\n", "new\n");
    const pending = ask({ ...edit, phase: "after", content: [change] });
    const request = await nextRequest(daemon, session, 0);
    await writeFile(join(daemon.dir, "a.ts"), "typed by the user\n");
    await press(daemon, session, request.id, "revert");
    expect(await pending).toEqual({ allowed: false, reverted: false });
    expect(await readFile(join(daemon.dir, "a.ts"), "utf8")).toBe("typed by the user\n");
  });

  test("an edit the tool already had reviewed is not reviewed again when its call completes", async () => {
    const { daemon, session, agent } = await setup();
    daemon.settings.setApprovals(session, "review");
    daemon.sessions.input(session, "cli", "go");
    await agent.started(1);
    const ask = (agent.created[0] ?? agent.loaded[0])?.requestPermission;
    if (!ask) throw new Error("no hook");
    const change = await written(daemon, "a.ts", "old\n", "new\n");
    const pending = ask({ ...edit, phase: "after", content: [change] });
    await press(daemon, session, (await nextRequest(daemon, session, 0)).id, "keep");
    await pending;
    const { onUpdate } = agent.prompts[0] ?? { onUpdate: () => {} };
    onUpdate({
      type: "tool_call",
      toolCallId: "tc",
      title: "edit",
      kind: "edit",
      status: "pending",
    });
    onUpdate({
      type: "tool_call_update",
      toolCallId: "tc",
      status: "completed",
      content: [change],
    });
    agent.release();
    await daemon.sessions.idle(session);
    await Bun.sleep(20);
    expect(requests(daemon, session)).toHaveLength(1);
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);
  });
});

describe("review mode, edits an ACP harness made", () => {
  /** A turn in which an ACP harness ("claude") reports completed edits, without asking. */
  async function claudeTurn(
    daemon: TestDaemon,
    session: string,
    agent: FakeAgent,
    changes: DiffContent[],
  ): Promise<void> {
    const next = agent.prompts.length + 1;
    daemon.sessions.input(session, "cli", "change things");
    await agent.started(next);
    const { onUpdate } = agent.prompts[next - 1] ?? { onUpdate: () => {} };
    changes.forEach((change, i) => {
      const toolCallId = `claude:t${next}-${i}`;
      onUpdate({ type: "tool_call", toolCallId, title: "Edit", kind: "edit", status: "pending" });
      onUpdate({
        type: "tool_call_update",
        toolCallId,
        status: "completed",
        content: [change],
      });
    });
    agent.release();
    await daemon.sessions.idle(session);
  }

  test("each completed edit gets a Keep / Revert card; no card after the turn", async () => {
    const { daemon, session, agent } = await setup();
    daemon.settings.setApprovals(session, "review");
    const a = join(daemon.dir, "a.ts");
    const b = join(daemon.dir, "b.ts");
    await writeFile(a, "a2\n");
    await writeFile(b, "b2\n");
    await claudeTurn(daemon, session, agent, [
      { type: "diff", path: a, oldText: "a1\n", newText: "a2\n" },
      { type: "diff", path: b, oldText: "b1\n", newText: "b2\n" },
    ]);
    await until(() => requests(daemon, session).length === 2, "both cards");
    const [first, second] = requests(daemon, session);
    expect(first).toMatchObject({
      agent: "claude",
      title: "Edited a.ts — keep it?",
      kind: "edit",
      content: [{ type: "diff", path: a, oldText: "a1\n", newText: "a2\n" }],
      options: [{ id: "keep" }, { id: "revert" }],
    });
    expect(first).not.toHaveProperty("toolCallId");
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);

    // Keep one, revert the other with a note: the agent hears both on its next prompt.
    if (first?.type !== "permission_request" || second?.type !== "permission_request") return;
    await press(daemon, session, first.id, "keep");
    await press(daemon, session, second.id, "revert", "b was fine, a was wrong");
    await until(() => events(daemon, session).some((e) => e.type === "edits_reverted"), "revert");
    expect(await readFile(a, "utf8")).toBe("a2\n");
    expect(await readFile(b, "utf8")).toBe("b1\n");
    expect(events(daemon, session).findLast((e) => e.type === "edits_reverted")).toEqual({
      type: "edits_reverted",
      paths: ["b.ts"],
    });
    daemon.sessions.input(session, "cli", "what now?");
    await agent.started(2);
    expect(agent.prompts[1]?.pendingEvents).toEqual([
      {
        type: "user_note",
        note: "b was fine, a was wrong",
        about: "Edited b.ts — keep it? → Revert",
      },
      { type: "edits_reverted", paths: ["b.ts"] },
    ]);
    agent.release();
    await daemon.sessions.idle(session);
  });

  test("a file changed since is skipped; created files are deleted", async () => {
    const { daemon, session, agent } = await setup();
    daemon.settings.setApprovals(session, "review");
    const a = join(daemon.dir, "a.ts");
    const c = join(daemon.dir, "new.ts");
    await writeFile(a, "typed\n");
    await writeFile(c, "c\n");
    await claudeTurn(daemon, session, agent, [
      { type: "diff", path: a, oldText: "a1\n", newText: "a2\n" },
      { type: "diff", path: c, newText: "c\n" },
    ]);
    await until(() => requests(daemon, session).length === 2, "both cards");
    for (const request of requests(daemon, session)) {
      if (request.type === "permission_request") await press(daemon, session, request.id, "revert");
    }
    await until(
      () => events(daemon, session).filter((e) => e.type === "edits_reverted").length === 2,
      "reverts",
    );
    expect(await readFile(a, "utf8")).toBe("typed\n");
    expect(existsSync(c)).toBe(false);
    const reverts = events(daemon, session).filter((e) => e.type === "edits_reverted");
    expect(reverts).toEqual([
      { type: "edits_reverted", paths: [], skipped: ["a.ts"] },
      { type: "edits_reverted", paths: ["new.ts"] },
    ]);
  });
});

describe("edits mode, the agent's own tools", () => {
  test("edits go ahead without a second question; bash and unknown actions ask", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals edits");
    expect(await ask({ title: "write a", kind: "edit", content: [diff] })).toEqual({
      allowed: true,
    });
    const pending = ask({ title: "make build", kind: "execute" });
    const request = await nextRequest(daemon, session, 0);
    await press(daemon, session, request.id, "reject");
    expect(await pending).toEqual({ allowed: false });
  });
});

describe("notes with an answer", () => {
  test("the neo hook gets the note back; the answered card shows it", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    const pending = ask({ title: "make build", kind: "execute" });
    const request = await nextRequest(daemon, session, 0);
    await press(daemon, session, request.id, "allow", "  use -j4  ");
    expect(await pending).toEqual({ allowed: true, note: "use -j4" });
    expect(events(daemon, session).findLast((e) => e.type === "permission_resolved")).toMatchObject(
      { option: "allow", note: "use -j4" },
    );
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `p-${request.id}`);
    expect(JSON.stringify(item?.component)).toContain("Allow · “use -j4”");
    // The tool tells the model itself: no event for the next prompt.
    expect(events(daemon, session).some((e) => e.type === "user_note")).toBe(false);

    const refused = ask({ title: "rm -r build", kind: "execute" });
    const second = await nextRequest(daemon, session, 1);
    await press(daemon, session, second.id, "reject", "just clean the cache");
    expect(await refused).toEqual({ allowed: false, note: "just clean the cache" });
  });

  test("an empty note is no note; a note must be text", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    const pending = ask({ title: "ls", kind: "execute" });
    const request = await nextRequest(daemon, session, 0);
    await press(daemon, session, request.id, "allow", "   ");
    expect(await pending).toEqual({ allowed: true });
    expect(
      daemon.actions.run(
        { session, ref: "x", action: "suite.permit", args: { id: "p", option: "a", note: 3 } },
        "t",
      ),
    ).rejects.toThrow("note must be a string");
  });

  test("a note on a choice reaches the agent with its next prompt", async () => {
    const { daemon, session, agent } = await setup();
    daemon.settings.setApprovals(session, "edits");
    const a = join(daemon.dir, "a.ts");
    await writeFile(a, "a2\n");
    daemon.sessions.input(session, "cli", "change");
    await agent.started(1);
    const { onUpdate } = agent.prompts[0] ?? { onUpdate: () => {} };
    onUpdate({
      type: "tool_call",
      toolCallId: "e",
      title: "edit",
      kind: "edit",
      status: "pending",
    });
    onUpdate({
      type: "tool_call_update",
      toolCallId: "e",
      status: "completed",
      content: [{ type: "diff", path: a, oldText: "a1\n", newText: "a2\n" }],
    });
    agent.release();
    await daemon.sessions.idle(session);
    const card = events(daemon, session).findLast((e) => e.type === "choice");
    if (card?.type !== "choice") throw new Error("no card");
    await daemon.actions.run(
      {
        session,
        ref: "x",
        action: "suite.choose",
        args: { id: card.id, option: "revert", note: "keep the old name" },
      },
      "t",
    );
    await until(() => events(daemon, session).some((e) => e.type === "edits_reverted"), "revert");
    expect(events(daemon, session).findLast((e) => e.type === "choice_made")).toMatchObject({
      option: "revert",
      note: "keep the old name",
    });
    daemon.sessions.input(session, "cli", "again");
    await agent.started(2);
    expect(agent.prompts[1]?.pendingEvents).toEqual([
      { type: "user_note", note: "keep the old name", about: "Edited 1 file: a.ts → Revert all" },
      { type: "edits_reverted", paths: ["a.ts"] },
    ]);
    agent.release();
    await daemon.sessions.idle(session);
  });
});

describe("review after the turn", () => {
  /** Runs a turn in which the agent "edits" files: writes them and reports the diffs. */
  async function editTurn(
    daemon: TestDaemon,
    session: string,
    agent: FakeAgent,
    edits: { path: string; before?: string; after: string }[],
  ): Promise<void> {
    const next = agent.prompts.length + 1;
    daemon.sessions.input(session, "cli", "change things");
    await agent.started(next);
    const { onUpdate } = agent.prompts[next - 1] ?? { onUpdate: () => {} };
    edits.forEach((edit, i) => {
      const toolCallId = `e${next}-${i}`;
      onUpdate({ type: "tool_call", toolCallId, title: "edit", kind: "edit", status: "pending" });
      onUpdate({
        type: "tool_call_update",
        toolCallId,
        status: "completed",
        content: [{ type: "diff", path: edit.path, oldText: edit.before, newText: edit.after }],
      });
    });
    agent.release();
    await daemon.sessions.idle(session);
  }

  function card(daemon: TestDaemon, session: string): Choice {
    const choice = events(daemon, session).findLast((e) => e.type === "choice");
    if (choice?.type !== "choice") throw new Error("no review card");
    return choice;
  }

  async function pick(daemon: TestDaemon, session: string, option: string): Promise<void> {
    const { id } = card(daemon, session);
    await daemon.actions.run(
      { session, ref: "x", action: "suite.choose", args: { id, option } },
      "t",
    );
  }

  async function files(daemon: TestDaemon): Promise<{ a: string; b: string; c: string }> {
    const a = join(daemon.dir, "a.ts");
    const b = join(daemon.dir, "b.ts");
    const c = join(daemon.dir, "notes.md");
    await writeFile(a, "a1\n");
    await writeFile(b, "b1\n");
    return { a, b, c };
  }

  test("nothing to review outside edits mode, or without edits", async () => {
    const { daemon, session, agent } = await setup();
    const { a } = await files(daemon);
    await editTurn(daemon, session, agent, [{ path: a, before: "a1\n", after: "a2\n" }]);
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);
    daemon.settings.setApprovals(session, "edits");
    daemon.sessions.input(session, "cli", "just talk");
    await agent.started(2);
    agent.release();
    await daemon.sessions.idle(session);
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);
  });

  test("an edit the user was asked about is not reviewed again", async () => {
    const { daemon, session, agent } = await setup();
    const { a } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    daemon.sessions.input(session, "cli", "go");
    await agent.started(1);
    const { onUpdate } = agent.prompts[0] ?? { onUpdate: () => {} };
    daemon.store.append(session, {
      type: "permission_request",
      id: "p_x",
      agent: "claude",
      title: "Edit a.ts",
      toolCallId: "claude:t",
      options: [],
    });
    onUpdate({
      type: "tool_call",
      toolCallId: "claude:t",
      title: "Edit",
      kind: "edit",
      status: "pending",
    });
    onUpdate({
      type: "tool_call_update",
      toolCallId: "claude:t",
      status: "completed",
      content: [{ type: "diff", path: a, oldText: "a1\n", newText: "a2\n" }],
    });
    agent.release();
    await daemon.sessions.idle(session);
    expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);
  });

  test("the card lists the files; keep leaves them", async () => {
    const { daemon, session, agent } = await setup();
    const { a, b, c } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    await editTurn(daemon, session, agent, [
      { path: a, before: "a1\n", after: "a2\n" },
      { path: b, before: "b1\n", after: "b2\n" },
      { path: c, after: "new\n" },
    ]);
    expect(card(daemon, session)).toMatchObject({
      title: "Edited 3 files: a.ts, b.ts, notes.md",
      options: [
        { id: "keep", name: "Keep" },
        { id: "revert", name: "Revert all" },
        { id: `revert:${a}`, name: "Revert a.ts" },
        { id: `revert:${b}`, name: "Revert b.ts" },
        { id: `revert:${c}`, name: "Revert notes.md" },
      ],
    });
    await pick(daemon, session, "keep");
    expect(events(daemon, session).some((e) => e.type === "edits_reverted")).toBe(false);
  });

  test("revert all restores the first oldText and deletes created files, and tells the agent", async () => {
    const { daemon, session, agent } = await setup();
    const { a, b, c } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    // The agent edits a twice and creates notes.md; the files on disk end as the last newText.
    await writeFile(a, "a3\n");
    await writeFile(b, "b2\n");
    await writeFile(c, "new\n");
    await editTurn(daemon, session, agent, [
      { path: a, before: "a1\n", after: "a2\n" },
      { path: b, before: "b1\n", after: "b2\n" },
      { path: a, before: "a2\n", after: "a3\n" },
      { path: c, after: "new\n" },
    ]);
    expect(card(daemon, session).title).toBe("Edited 3 files: a.ts, b.ts, notes.md");
    await pick(daemon, session, "revert");
    await until(
      () => events(daemon, session).some((e) => e.type === "edits_reverted"),
      "the revert",
    );
    expect(await readFile(a, "utf8")).toBe("a1\n");
    expect(await readFile(b, "utf8")).toBe("b1\n");
    expect(existsSync(c)).toBe(false);
    expect(events(daemon, session).findLast((e) => e.type === "edits_reverted")).toEqual({
      type: "edits_reverted",
      paths: ["a.ts", "b.ts", "notes.md"],
    });
    const notice = daemon.views.get(session).snapshot().items.at(-1);
    expect(JSON.stringify(notice)).toContain("reverted a.ts, b.ts, notes.md");

    // The next prompt carries it.
    daemon.sessions.input(session, "cli", "what now?");
    await agent.started(2);
    expect(agent.prompts[1]?.pendingEvents).toEqual([
      { type: "edits_reverted", paths: ["a.ts", "b.ts", "notes.md"] },
    ]);
    agent.release();
    await daemon.sessions.idle(session);
  });

  test("revert one file asks again about the rest", async () => {
    const { daemon, session, agent } = await setup();
    const { a, b } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    await writeFile(a, "a2\n");
    await writeFile(b, "b2\n");
    await editTurn(daemon, session, agent, [
      { path: a, before: "a1\n", after: "a2\n" },
      { path: b, before: "b1\n", after: "b2\n" },
    ]);
    const first = card(daemon, session);
    await pick(daemon, session, `revert:${b}`);
    await until(
      () => events(daemon, session).some((e) => e.type === "edits_reverted"),
      "the revert",
    );
    expect(await readFile(a, "utf8")).toBe("a2\n");
    expect(await readFile(b, "utf8")).toBe("b1\n");
    await until(() => card(daemon, session).id !== first.id, "the second card");
    expect(card(daemon, session)).toMatchObject({
      title: "Edited 1 file: a.ts",
      options: [{ id: "keep" }, { id: "revert" }],
    });
    expect(events(daemon, session).findLast((e) => e.type === "edits_reverted")).toEqual({
      type: "edits_reverted",
      paths: ["b.ts"],
    });
  });

  test("a file that changed since the turn is skipped, and the card says so", async () => {
    const { daemon, session, agent } = await setup();
    const { a, b } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    await writeFile(a, "a2\n");
    await writeFile(b, "b2\n");
    await editTurn(daemon, session, agent, [
      { path: a, before: "a1\n", after: "a2\n" },
      { path: b, before: "b1\n", after: "b2\n" },
    ]);
    await writeFile(a, "the user typed here\n");
    await pick(daemon, session, "revert");
    await until(
      () => events(daemon, session).some((e) => e.type === "edits_reverted"),
      "the revert",
    );
    expect(await readFile(a, "utf8")).toBe("the user typed here\n");
    expect(await readFile(b, "utf8")).toBe("b1\n");
    expect(events(daemon, session).findLast((e) => e.type === "edits_reverted")).toEqual({
      type: "edits_reverted",
      paths: ["b.ts"],
      skipped: ["a.ts"],
    });
    const notice = daemon.views.get(session).snapshot().items.at(-1);
    expect(JSON.stringify(notice)).toContain("left a.ts (changed since)");
  });

  test("line endings and a BOM survive a revert", async () => {
    const { daemon, session, agent } = await setup();
    const { a } = await files(daemon);
    daemon.settings.setApprovals(session, "edits");
    await writeFile(a, "﻿x\r\ny2\r\n");
    // The edit tool reports LF text without the BOM.
    await editTurn(daemon, session, agent, [{ path: a, before: "x\ny\n", after: "x\ny2\n" }]);
    await pick(daemon, session, "revert");
    await until(
      () => events(daemon, session).some((e) => e.type === "edits_reverted"),
      "the revert",
    );
    expect(await readFile(a, "utf8")).toBe("﻿x\r\ny\r\n");
  });

  test("a guest's direct turn gets a card too", async () => {
    const guest = {
      title: "Fake",
      command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
      permissions: "ask" as const,
    };
    const daemon = await startTestDaemon({ agent: new FakeAgent("neo"), guests: { fake: guest } });
    cleanups.push(() => daemon.stop());
    const session = daemon.sessions.create({}).id;
    daemon.settings.setApprovals(session, "edits");
    daemon.sessions.input(session, "cli", "/fake edit");
    await daemon.sessions.idle(session);
    expect(card(daemon, session)).toMatchObject({ title: "Edited 1 file: /tmp/a.txt" });
    // /tmp/a.txt isn't there to put back.
    await pick(daemon, session, "revert");
    await until(
      () => events(daemon, session).some((e) => e.type === "edits_reverted"),
      "the revert",
    );
    expect(events(daemon, session).findLast((e) => e.type === "edits_reverted")).toEqual({
      type: "edits_reverted",
      paths: [],
      skipped: ["/tmp/a.txt"],
    });
  });
});

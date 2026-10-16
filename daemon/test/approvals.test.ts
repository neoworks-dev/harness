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
  const agent = new FakeAgent("pi");
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
): Promise<void> {
  await daemon.actions.run(
    { session, ref: "x", action: "suite.permit", args: { id, option } },
    "t",
  );
}

const diff: DiffContent = { type: "diff", path: "/w/a.ts", oldText: "a\n", newText: "b\n" };

describe("the agent's own tools by approvals mode", () => {
  test("auto never asks", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals auto");
    expect(await ask({ title: "rm -rf x", kind: "execute" })).toBe(true);
    expect(await ask({ title: "write a", kind: "edit", content: [diff] })).toBe(true);
    expect(requests(daemon, session)).toEqual([]);
  });

  test("edits goes ahead with edits and asks for the rest", async () => {
    const { daemon, session, agent } = await setup();
    const ask = await hookOf(daemon, session, agent);
    await daemon.commands.run(session, "/approvals edits");
    expect(await ask({ title: "write a", kind: "edit", content: [diff] })).toBe(true);
    expect(requests(daemon, session)).toEqual([]);
    const pending = ask({ title: "ls", kind: "execute", toolCallId: "tc1" });
    await until(() => requests(daemon, session).length === 1, "the question");
    const [request] = requests(daemon, session);
    if (request?.type !== "permission_request") throw new Error("no request");
    expect(request).toMatchObject({
      agent: "pi",
      title: "ls",
      kind: "execute",
      toolCallId: "tc1",
      options: [
        { id: "allow", name: "Allow", kind: "allow_once" },
        { id: "always", name: "Allow for this session", kind: "allow_always" },
        { id: "reject", name: "Reject", kind: "reject_once" },
      ],
    });
    await press(daemon, session, request.id, "reject");
    expect(await pending).toBe(false);
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
      label: "pi asks: edit a.ts",
      state: "pending",
      component: {
        type: "stack",
        children: [{ type: "diff", path: "/w/a.ts" }, { type: "row" }],
      },
    });
    await press(daemon, session, request.id, "always");
    expect(await pending).toBe(true);
    expect(item()).toMatchObject({ state: "done", component: { type: "text" } });

    // Edits are remembered for the session; commands and other sessions still ask.
    expect(await ask({ title: "edit b.ts", kind: "edit", content: [diff] })).toBe(true);
    expect(requests(daemon, session)).toHaveLength(1);
    const bash = ask({ title: "ls", kind: "execute" });
    await until(() => requests(daemon, session).length === 2, "the second question");
    const second = requests(daemon, session)[1];
    if (second?.type !== "permission_request") throw new Error("no request");
    await press(daemon, session, second.id, "allow");
    expect(await bash).toBe(true);

    // A cancelled turn drops its open question: nobody answered, so no.
    const controller = new AbortController();
    const aborted = ask({ title: "pwd", kind: "execute" }, controller.signal);
    await until(() => requests(daemon, session).length === 3, "the third question");
    controller.abort();
    expect(await aborted).toBe(false);
    const last = events(daemon, session).findLast((e) => e.type === "permission_resolved");
    expect(last).not.toHaveProperty("option");
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
    const daemon = await startTestDaemon({ agent: new FakeAgent("pi"), guests: { fake: guest } });
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

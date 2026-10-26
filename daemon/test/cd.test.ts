import { afterEach, expect, test } from "bun:test";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
});

async function setup(): Promise<{ daemon: TestDaemon; agent: FakeAgent; root: string }> {
  const root = await mkdtemp(join(tmpdir(), "neo-cd-"));
  await mkdir(join(root, "app", "src"), { recursive: true });
  const agent = new FakeAgent("neo");
  const daemon = await startTestDaemon({ agent });
  cleanups.push(async () => {
    await daemon.stop();
    await rm(root, { recursive: true, force: true });
  });
  return { daemon, agent, root };
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

async function turn(daemon: TestDaemon, agent: FakeAgent, session: string): Promise<void> {
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(agent.prompts.length + 1);
  agent.release();
  await daemon.sessions.idle(session);
}

test("/cd moves the session, and its agent loads again there with a note", async () => {
  const { daemon, agent, root } = await setup();
  const session = daemon.sessions.create({ cwd: root }).id;
  await turn(daemon, agent, session);
  expect(agent.created.at(-1)?.cwd).toBe(root);

  await daemon.commands.run(session, "/cd app/src");
  const dir = join(root, "app", "src");
  expect(daemon.sessions.info(session)?.cwd).toBe(dir);
  expect(agent.closed).toEqual(["other"]);
  const [notice, note] = events(daemon, session).slice(-2);
  expect(notice).toEqual({ type: "notice", text: `moved to ${dir}` });
  expect(note).toMatchObject({ type: "user_note", about: "/cd app/src" });

  // The next turn loads the agent's session, with its history, in the new directory.
  await turn(daemon, agent, session);
  expect(agent.loaded.at(-1)?.cwd).toBe(dir);

  // Relative to where it is now.
  await daemon.commands.run(session, "/cd ../..");
  expect(daemon.sessions.info(session)?.cwd).toBe(root);
});

test("/cd alone says where the session is, and a missing directory is refused", async () => {
  const { daemon, root } = await setup();
  const session = daemon.sessions.create({ cwd: root }).id;
  await daemon.commands.run(session, "/cd");
  expect(events(daemon, session).at(-1)).toEqual({
    type: "notice",
    text: `in ${root}`,
    tone: "muted",
  });
  const refused = await daemon.commands.run(session, "/cd nowhere")?.catch((e: unknown) => e);
  expect(String(refused)).toContain(`no directory ${join(root, "nowhere")}`);
  expect(daemon.sessions.info(session)?.cwd).toBe(root);
});

test("/cd waits for the turn to end", async () => {
  const { daemon, agent, root } = await setup();
  const session = daemon.sessions.create({ cwd: root }).id;
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(1);
  const refused = await daemon.commands.run(session, "/cd app")?.catch((e: unknown) => e);
  expect(String(refused)).toContain("wait until the turn ends");
  agent.release();
  await daemon.sessions.idle(session);
  expect(daemon.sessions.info(session)?.cwd).toBe(root);
});

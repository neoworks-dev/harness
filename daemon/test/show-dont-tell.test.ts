import { afterEach, expect, test } from "bun:test";
import type { ContextEvent } from "@neoworks-dev/neo/agent";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
});

async function setup(): Promise<{ daemon: TestDaemon; agent: FakeAgent }> {
  const agent = new FakeAgent("neo");
  const daemon = await startTestDaemon({ agent });
  cleanups.push(() => daemon.stop());
  return { daemon, agent };
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

type ShowDontTell = Extract<ContextEvent, { type: "show_dont_tell" }>;

/** Runs a turn and returns the show_dont_tell events its prompt carried. */
async function turn(
  daemon: TestDaemon,
  agent: FakeAgent,
  session: string,
): Promise<ShowDontTell[]> {
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(agent.prompts.length + 1);
  agent.release();
  await daemon.sessions.idle(session);
  const pending = agent.prompts.at(-1)?.pendingEvents ?? [];
  return pending.filter((e): e is ShowDontTell => e.type === "show_dont_tell");
}

test("Show, don't tell. reaches each agent once, with the UI library when it is on", async () => {
  const { daemon, agent } = await setup();
  const session = daemon.sessions.create({}).id;
  // Off by default: nothing to tell.
  expect(await turn(daemon, agent, session)).toEqual([]);

  await daemon.commands.run(session, "/settings show-dont-tell on");
  const told = await turn(daemon, agent, session);
  expect(told).toHaveLength(1);
  expect(told[0]?.on).toBe(true);
  expect(told[0]?.library).toContain("neoui");
  // Told once: the next prompt doesn't repeat it.
  expect(await turn(daemon, agent, session)).toEqual([]);

  // A session that hasn't heard of it yet is told with its first prompt.
  const other = daemon.sessions.create({}).id;
  expect((await turn(daemon, agent, other)).map((e) => e.on)).toEqual([true]);

  // Without a value it flips; off is said without the library.
  await daemon.commands.run(session, "/settings show-dont-tell");
  expect(await turn(daemon, agent, session)).toEqual([{ type: "show_dont_tell", on: false }]);
  const notices: string[] = [];
  for (const e of events(daemon, session)) if (e.type === "notice") notices.push(e.text);
  expect(notices).toEqual([
    "Show, don't tell. on, for every session. Agents hear of it with their next prompt.",
    "Show, don't tell. off, for every session.",
  ]);
});

test("/settings without a toggle shows them with their state, and picking one flips it", async () => {
  const { daemon, agent } = await setup();
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/settings");
  const choice = events(daemon, session).find((e) => e.type === "choice");
  if (choice?.type !== "choice") throw new Error("no choice was shown");
  expect(choice.options.map((o) => o.name)).toEqual([
    "Show, don't tell. · off",
    "Side-by-side diffs · off",
  ]);
  expect(choice.options[0]?.description).toContain("UI more than text");

  const args = { id: choice.id, option: "show-dont-tell" };
  await daemon.actions.run({ session, ref: "x", action: "neo.choose", args }, "test");
  await Bun.sleep(10);
  expect((await turn(daemon, agent, session)).map((e) => e.on)).toEqual([true]);
});

test("side-by-side diffs is a toggle for the apps: they hear of it, the agents don't", async () => {
  const { daemon, agent } = await setup();
  const session = daemon.sessions.create({}).id;
  const heard: unknown[] = [];
  daemon.settings.onToggle((settings) => heard.push(settings));
  await daemon.commands.run(session, "/settings split-diffs on");
  expect(heard).toEqual([{ showDontTell: false, splitDiffs: true }]);
  expect(daemon.settings.userSettings().splitDiffs).toBe(true);
  const notices: string[] = [];
  for (const e of events(daemon, session)) if (e.type === "notice") notices.push(e.text);
  expect(notices).toEqual(["Side-by-side diffs on, for every session."]);
  expect(await turn(daemon, agent, session)).toEqual([]);
});

test("/settings rejects what it doesn't know", async () => {
  const { daemon } = await setup();
  const session = daemon.sessions.create({}).id;
  for (const bad of ["/settings nope", "/settings show-dont-tell maybe", "/settings a b c"]) {
    let error: unknown;
    try {
      await daemon.commands.run(session, bad);
    } catch (err) {
      error = err;
    }
    expect(String(error)).toContain("usage: /settings");
  }
});

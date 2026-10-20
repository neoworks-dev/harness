import { afterEach, expect, test } from "bun:test";
import type { SessionEvent } from "@suite/protocol/types";
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

function notices(daemon: TestDaemon, session: string): string[] {
  const texts: string[] = [];
  for (const e of events(daemon, session)) if (e.type === "notice") texts.push(e.text);
  return texts;
}

/** Runs a turn so the agent session exists, and returns the safe search the daemon gave it. */
async function safeSearchOf(
  daemon: TestDaemon,
  agent: FakeAgent,
  session: string,
): Promise<() => string | undefined> {
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(agent.prompts.length + 1);
  agent.release();
  await daemon.sessions.idle(session);
  const getter = agent.created.at(-1)?.safeSearch;
  if (!getter) throw new Error("the daemon gave the agent no safe search");
  return getter;
}

test("/safesearch sets it for every session, read at each search", async () => {
  const { daemon, agent } = await setup();
  const first = daemon.sessions.create({}).id;
  const safeSearch = await safeSearchOf(daemon, agent, first);
  // Nothing chosen: the harness keeps its configured default.
  expect(safeSearch()).toBeUndefined();

  await daemon.commands.run(first, "/safesearch strict");
  expect(safeSearch()).toBe("strict");
  expect(notices(daemon, first)).toEqual([
    "Safe search: strict, for web searches in every session.",
  ]);

  const second = daemon.sessions.create({}).id;
  expect((await safeSearchOf(daemon, agent, second))()).toBe("strict");
  // It is not a session's setting, so no session logs it.
  expect(events(daemon, second).some((e) => e.type === "setting_changed")).toBe(false);
});

test("/safesearch without a value asks, and a bad value is refused", async () => {
  const { daemon } = await setup();
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/safesearch off");
  await daemon.commands.run(session, "/safesearch");
  const choice = events(daemon, session).findLast((e) => e.type === "choice");
  if (choice?.type !== "choice") throw new Error("no choice card");
  expect(choice.title).toBe("Safe search for web searches (now: off)");
  expect(choice.current).toBe("off");
  expect(choice.options.map((o) => o.id)).toEqual(["strict", "moderate", "off"]);

  await daemon.actions.run(
    { session, ref: "x", action: "suite.choose", args: { id: choice.id, option: "moderate" } },
    "t",
  );
  await Bun.sleep(5); // the card's answer is applied after the action returns
  expect(daemon.settings.safeSearch()).toBe("moderate");

  let refused = "";
  await daemon.commands.run(session, "/safesearch none")?.catch((error: unknown) => {
    refused = String(error);
  });
  expect(refused).toContain("usage: /safesearch [strict|moderate|off]");
});

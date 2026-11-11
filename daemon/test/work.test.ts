import { afterEach, expect, test } from "bun:test";
import { servicesFor } from "../src/services.ts";
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

/** Starts a conversation in a new session and returns what neo was given for it. */
async function started(
  daemon: TestDaemon,
  agent: FakeAgent,
): Promise<{ work?: string } | undefined> {
  const session = daemon.sessions.create({}).id;
  daemon.sessions.input(session, "cli", "hi");
  await agent.started(agent.prompts.length + 1);
  agent.release();
  await daemon.sessions.idle(session);
  return [...agent.created, ...agent.loaded].at(-1);
}

test("until the user says what they do with neo, it is coding, and apps can tell", async () => {
  const { daemon, agent } = await setup();
  expect(daemon.settings.work()).toBeUndefined();
  expect(daemon.settings.userSettings().work).toBeUndefined();
  expect((await started(daemon, agent))?.work).toBeUndefined();
});

test("work on the computer reaches neo's new conversations and the apps", async () => {
  const { daemon, agent } = await setup();
  const session = daemon.sessions.create({}).id;
  const told: (string | undefined)[] = [];
  daemon.settings.onToggle((settings) => told.push(settings.work));
  await daemon.commands.run(session, "/settings work computer");
  expect(daemon.settings.work()).toBe("computer");
  expect(told).toEqual(["computer"]);
  expect((await started(daemon, agent))?.work).toBe("computer");
});

test("the language servers run for coding only, the agent's own tools for every kind of work", () => {
  expect(servicesFor("coding").length).toBe(servicesFor("computer").length + 1);
  expect(servicesFor("computer")).toHaveLength(1);
});

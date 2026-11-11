// The session's agent going on by itself: Claude Code's follow-up once a command or subagent it
// ran in the background ended, which ACP v1 sends outside any prompt.
import { afterEach, expect, test } from "bun:test";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function setup(): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({
    agent: new FakeAgent("main"),
    guests: {
      fake: {
        title: "Fake",
        command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
        permissions: "ask",
      },
    },
  });
  cleanups.push(() => daemon.stop());
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/harness fake");
  return { daemon, session };
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

test("a conversation cleared away stops: its agent doesn't go on in the session afterwards", async () => {
  const { daemon, session } = await setup();
  daemon.sessions.input(session, "cli", "later 400");
  await until(
    () => events(daemon, session).some((e) => e.type === "turn_ended"),
    "the turn to end",
  );
  await daemon.sessions.idle(session);
  // /clear before the old conversation goes on (Claude Code's idle compaction, an hour on).
  daemon.sessions.clearContext(session);
  await Bun.sleep(700);
  const all = events(daemon, session);
  expect(all.some((e) => e.type === "user_message" && e.app === "background")).toBe(false);
  expect(all.some((e) => e.type === "agent_message_chunk" && e.text === "The build passed.")).toBe(false);
});

test("what the agent does after its turn ended is a turn of its own, over at its result", async () => {
  const { daemon, session } = await setup();
  daemon.sessions.input(session, "cli", "later");
  const turns = (): SessionEvent[] =>
    events(daemon, session).filter((e) => e.type === "turn_ended");
  await until(() => turns().length === 2, "the agent's own turn to end");
  await daemon.sessions.idle(session);

  const all = events(daemon, session);
  const opened = all.findIndex((e) => e.type === "user_message" && e.app === "background");
  expect(all[opened]).toMatchObject({ text: "Fake went on by itself" });
  const said = all.findIndex(
    (e) => e.type === "agent_message_chunk" && e.text === "The build passed.",
  );
  const ended = all.findLastIndex((e) => e.type === "turn_ended");
  expect(opened).toBeGreaterThan(0);
  expect(said).toBeGreaterThan(opened);
  expect(ended).toBeGreaterThan(said);
  expect(all[ended]).toEqual({ type: "turn_ended", stopReason: "end_turn" });
  expect(daemon.sessions.info(session)?.state).toBe("idle");

  // The next prompt runs as usual.
  daemon.sessions.input(session, "cli", "commands");
  await until(() => turns().length === 3, "the next turn");
  expect(
    events(daemon, session).some((e) => e.type === "agent_message_chunk" && e.text === "listed"),
  ).toBe(true);
});

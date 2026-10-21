// The slash commands a guest harness lists (ACP available_commands_update) reach the session's
// log as commands_available, and `/name args` goes to the guest as the user's text.
import { afterEach, expect, test } from "bun:test";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { slashCommands, type GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
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
  guest: Partial<GuestConfig> = {},
): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({
    agent: new FakeAgent("main"),
    guests: { fake: { ...FAKE, ...guest } },
  });
  cleanups.push(() => daemon.stop());
  return { daemon, session: daemon.sessions.create({}).id };
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

/** The commands_available events of a harness, in order, as lists of names. */
function lists(daemon: TestDaemon, session: string, harness = "fake"): string[][] {
  const out: string[][] = [];
  for (const event of events(daemon, session)) {
    if (event.type === "commands_available" && event.harness === harness) {
      out.push(event.commands.map((command) => command.name));
    }
  }
  return out;
}

async function say(daemon: TestDaemon, session: string, text: string): Promise<void> {
  daemon.sessions.input(session, "cli", text);
  await daemon.sessions.idle(session);
}

test("an adapter's commands are cleaned up: no slash, no blanks, no duplicates", () => {
  expect(
    slashCommands([
      { name: "review", description: "Review\n the changes", input: { hint: "branch or PR" } },
      { name: "/compact" },
      { name: "review", description: "again" },
      { name: "" },
      { name: "two words" },
      { description: "nameless" },
      "junk",
    ]),
  ).toEqual([
    { name: "review", description: "Review the changes", hint: "branch or PR" },
    { name: "compact" },
  ]);
  expect(slashCommands(undefined)).toEqual([]);
});

test("a guest's commands are logged when its conversation starts, and again only when they change", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, "/harness fake");
  expect(lists(daemon, session)).toEqual([]);

  await say(daemon, session, "hello");
  const found = events(daemon, session).find((event) => event.type === "commands_available");
  expect(found).toEqual({
    type: "commands_available",
    harness: "fake",
    commands: [
      { name: "review", description: "Review the current changes", hint: "branch or PR" },
      { name: "compact", description: "Summarise the conversation" },
      { name: "model", description: "Claude's own model picker" },
    ],
  });
  expect(lists(daemon, session)).toEqual([["review", "compact", "model"]]);

  // The same list again is not news; a new one replaces it.
  await say(daemon, session, "more");
  expect(lists(daemon, session)).toHaveLength(1);
  await say(daemon, session, "commands");
  expect(lists(daemon, session)).toEqual([["review", "compact", "model"], ["review"]]);
});

test("commands listed before session/new answers are kept for the guest", async () => {
  const { daemon, session } = await setup({ env: { FAKE_ACP_COMMANDS_FIRST: "1" } });
  await daemon.commands.run(session, "/harness fake");
  await say(daemon, session, "hello");
  expect(lists(daemon, session)).toEqual([["review", "compact", "model"]]);
});

test("/name args goes to the guest as text, and the daemon's own commands stay the daemon's", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, "/harness fake");
  await say(daemon, session, "/review main");
  const message = events(daemon, session).find((event) => event.type === "user_message");
  expect(message).toMatchObject({ text: "/review main" });
  const thought = events(daemon, session).find((event) => event.type === "agent_thought_chunk");
  expect(thought).toMatchObject({ text: expect.stringContaining("/review main") });

  // /model is answered by the daemon even though the guest lists one: it never reaches the guest.
  const before = events(daemon, session).filter((event) => event.type === "user_message").length;
  await daemon.commands.run(session, "/model fake large");
  expect(events(daemon, session).filter((event) => event.type === "user_message")).toHaveLength(
    before,
  );
});

test("a delegated subagent's commands are not the user's", async () => {
  const { daemon, session } = await setup();
  daemon.store.onAppend(({ session: id, event }) => {
    if (event.type !== "permission_request" || event.title !== "Start a subagent?") return;
    void Promise.resolve().then(() =>
      daemon.actions.run(
        { session: id, ref: "p", action: "suite.permit", args: { id: event.id, option: "allow" } },
        "test",
      ),
    );
  });
  await daemon.gateway.call("delegate", { agent: "fake", task: "hello" }, undefined, session);
  expect(lists(daemon, session)).toEqual([]);
});

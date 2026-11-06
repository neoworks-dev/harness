import { afterEach, expect, test } from "bun:test";
import { join } from "node:path";
import type { GuestConfig } from "../src/guests/guests.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
});

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

async function setup(guests: Record<string, GuestConfig>): Promise<TestDaemon> {
  const daemon = await startTestDaemon({ agent: new FakeAgent("main"), guests });
  cleanups.push(() => daemon.stop());
  return daemon;
}

test("a guest is installed when its program is on the PATH, or when it names none", async () => {
  const daemon = await setup({
    there: { ...FAKE, program: "sh", signedIn: () => true },
    missing: { ...FAKE, program: "no-such-program-for-neo", signedIn: () => false },
    plain: FAKE,
  });
  const listed = daemon.harnesses.list();
  expect(listed.map((h) => [h.name, h.installed, h.signedIn])).toEqual([
    ["there", true, true],
    ["missing", false, false],
    ["plain", true, undefined],
  ]);
});

test("only Claude Code's kind can work with neo's tools: two choices, the one in use marked", async () => {
  const daemon = await setup({ fake: { ...FAKE, adapter: "claude" }, other: FAKE });
  const [fake, other] = daemon.harnesses.list();
  expect(fake?.tools.map((t) => [t.id, t.current])).toEqual([
    ["own", true],
    ["neo", false],
  ]);
  expect(other?.tools.map((t) => t.id)).toEqual(["own"]);
  daemon.settings.setGuestTools("fake", "neo");
  expect(daemon.harnesses.list()[0]?.tools.find((t) => t.current)?.id).toBe("neo");
});

test("a conversation's first usage is what the agent starts with; the smallest one is kept", async () => {
  const daemon = await setup({ fake: { ...FAKE, adapter: "claude" } });
  const start = (): number | undefined => daemon.harnesses.list()[0]?.tools[0]?.startTokens;
  const conversation = (tokens: number[], messages = 1): void => {
    const session = daemon.sessions.create({}).id;
    for (let i = 0; i < messages; i += 1) {
      daemon.store.append(session, { type: "user_message", app: "cli", text: "hi" });
    }
    for (const contextUsed of tokens) {
      daemon.store.append(session, { type: "usage", agent: "fake", contextUsed });
    }
  };
  expect(start()).toBeUndefined();
  // Later usage in the same conversation doesn't count.
  conversation([20_000, 30_000]);
  expect(start()).toBe(20_000);
  expect(daemon.harnesses.list()[0]?.tools[0]?.measured).toBe(true);
  conversation([18_000]);
  conversation([25_000]);
  expect(start()).toBe(18_000);
  // Further along than the first message: more than the start.
  conversation([10_000], 2);
  expect(start()).toBe(18_000);
});

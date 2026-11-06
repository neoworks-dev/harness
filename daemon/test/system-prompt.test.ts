// The user's words in every agent's system prompt (/system-prompt), a guest's own system prompt
// with neo's core prompt after it, and neo's file tools in place of Claude Code's (/settings).
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import type { GuestConfig } from "../src/guests/guests.ts";
import { MAIN_PROMPT } from "../src/guests/prompt.ts";
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
  adapter: "claude",
};

async function setup(
  guest: Partial<GuestConfig> = {},
): Promise<{ daemon: TestDaemon; agent: FakeAgent }> {
  const agent = new FakeAgent("main");
  const daemon = await startTestDaemon({ agent, guests: { fake: { ...FAKE, ...guest } } });
  cleanups.push(() => daemon.stop());
  return { daemon, agent };
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

/** What the session's agent said since the last user message. */
function reply(daemon: TestDaemon, session: string): string {
  let text = "";
  for (const e of events(daemon, session)) {
    if (e.type === "user_message") text = "";
    if (e.type === "agent_message_chunk" && e.agent === undefined) text += e.text;
  }
  return text;
}

/** The JSON the fake replied (it says "Looking." before every answer). */
function answer(text: string): Record<string, unknown> {
  const parsed: Record<string, unknown> = JSON.parse(text.slice(text.indexOf("{")));
  return parsed;
}

/** What the fake guest of a new session was started with (its `prompts` report). */
async function prompts(daemon: TestDaemon): Promise<Record<string, unknown>> {
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/harness fake");
  daemon.sessions.input(session, "cli", "prompts");
  await daemon.sessions.idle(session);
  return answer(reply(daemon, session));
}

/** Whether the command was refused. */
async function refused(daemon: TestDaemon, session: string, command: string): Promise<boolean> {
  try {
    await daemon.commands.run(session, command);
    return false;
  } catch {
    return true;
  }
}

function notices(daemon: TestDaemon, session: string): string[] {
  const out: string[] = [];
  for (const e of events(daemon, session)) if (e.type === "notice") out.push(e.text);
  return out;
}

describe("the system prompt", () => {
  test("/system-prompt puts the user's words after every agent's prompt; clear takes them out", async () => {
    const { daemon, agent } = await setup();
    const session = daemon.sessions.create({}).id;
    await daemon.commands.run(session, "/system-prompt");
    expect(notices(daemon, session).at(-1)).toContain("No words of yours");

    await daemon.commands.run(session, "/system-prompt Answer in German.\nBe brief.");
    expect(daemon.settings.promptAddition()).toBe("Answer in German.\nBe brief.");
    expect(daemon.settings.userSettings().promptAddition).toBe("Answer in German.\nBe brief.");
    await daemon.commands.run(session, "/system-prompt");
    expect(notices(daemon, session).at(-1)).toContain("Answer in German.\nBe brief.");

    // neo's own sessions, from their start.
    daemon.sessions.input(session, "cli", "hi");
    await agent.started(1);
    agent.release();
    await daemon.sessions.idle(session);
    const started = [...agent.created, ...agent.loaded].at(-1);
    expect(started?.promptAddition).toBe("Answer in German.\nBe brief.");
    // A guest: after neo's words for it.
    expect((await prompts(daemon)).instructionsEnd).toBe("ver in German.\nBe brief.".slice(-20));

    await daemon.commands.run(session, "/system-prompt clear");
    expect(daemon.settings.promptAddition()).toBeUndefined();
    expect(daemon.settings.userSettings().promptAddition).toBeUndefined();
  });

  for (const adapter of ["claude", "codex"] as const) {
    test(`${adapter}: a guest keeps its own system prompt, neo's core prompt after it`, async () => {
      const { daemon } = await setup({ adapter, env: { CODEX_CONFIG: "{}" } });
      const session = daemon.sessions.create({}).id;
      await daemon.commands.run(session, "/system-prompt Be brief.");
      const got = await prompts(daemon);
      // Nothing in place of its own, Claude Code's or Codex's.
      expect(got.systemPrompt).toBeUndefined();
      expect(got.base).toBeUndefined();
      // The session's own harness: neo's prompt for a main agent, then the user's words.
      expect(got.instructionsEnd).toBe(`${MAIN_PROMPT}\n\nBe brief.`.slice(-20));
      expect(await refused(daemon, session, "/settings fake-prompt neo")).toBe(true);
    });
  }

  test("/settings asks for the user's words and groups a guest's settings", async () => {
    const { daemon } = await setup();
    const session = daemon.sessions.create({}).id;
    await daemon.commands.run(session, "/settings");
    const shown = (): Extract<SessionEvent, { type: "choice" }> => {
      const choice = events(daemon, session).findLast((e) => e.type === "choice");
      if (choice?.type !== "choice") throw new Error("no choice was shown");
      return choice;
    };
    const pick = (args: Record<string, unknown>): Promise<void> =>
      daemon.actions.run(
        { session, ref: "x", action: "neo.choose", args: { id: shown().id, ...args } },
        "test",
      );
    const words = shown().options.find((o) => o.id === "system-prompt");
    expect(words?.input).toEqual({ value: "", multiline: true });
    // The words it was given: in, and the card stays, showing them now.
    await pick({ option: "system-prompt", value: "Answer in German.\nBe brief." });
    expect(daemon.settings.promptAddition()).toBe("Answer in German.\nBe brief.");
    const now = shown().options.find((o) => o.id === "system-prompt");
    expect(now?.name).toBe("Your words in the system prompt · 2 lines");
    expect(now?.input?.value).toBe("Answer in German.\nBe brief.");
    // The view's button asks for them too.
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === `q-${shown().id}`);
    expect(JSON.stringify(item?.component)).toContain('"multiline":true');

    // The guest's settings are a group: picked, they show, after the way back.
    expect(shown().options.at(-1)).toMatchObject({ id: "fake", name: "Fake ›" });
    await pick({ option: "fake", stay: true });
    expect(shown().title).toBe("Settings › Fake");
    expect(shown().options.map((o) => o.name)).toEqual(["‹ Back", "Fake's file tools · own"]);
    await pick({ option: "fake-tools", stay: true });
    expect(daemon.settings.guestTools("fake")).toBe("neo");
    expect(shown().options[1]?.name).toBe("Fake's file tools · neo");
    await pick({ option: "back", stay: true });
    expect(shown().title).toBe("Settings (pick one to change it)");
    // Emptied, the words are out.
    await pick({ option: "system-prompt", value: "  " });
    expect(daemon.settings.promptAddition()).toBeUndefined();
    expect(events(daemon, session).some((e) => e.type === "choice_made")).toBe(false);
  });

  test("Codex's file tools aren't neo's to swap", async () => {
    const { daemon } = await setup({ adapter: "codex", env: { CODEX_CONFIG: "{}" } });
    const session = daemon.sessions.create({}).id;
    expect(await refused(daemon, session, "/settings fake-tools neo")).toBe(true);
  });

  test("neo's hashline file tools stand in for Claude Code's own", async () => {
    const { daemon } = await setup();
    const dir = mkdtempSync(join(tmpdir(), "neo-files-"));
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
    writeFileSync(join(dir, "a.txt"), "one\ntwo\n");
    const session = daemon.sessions.create({ cwd: dir }).id;
    await daemon.commands.run(session, "/harness fake");
    await daemon.commands.run(session, "/settings fake-tools neo");
    await daemon.commands.run(session, "/approvals auto");
    const run = async (task: string): Promise<Record<string, unknown>> => {
      daemon.sessions.input(session, "cli", task);
      await daemon.sessions.idle(session);
      return answer(reply(daemon, session));
    };
    const got = await run("prompts");
    expect(got.disallowed).toEqual([
      "Agent",
      "Task",
      "Read",
      "Edit",
      "MultiEdit",
      "Write",
      "NotebookEdit",
    ]);
    expect(got.allow).toEqual(["mcp__neo__read", "mcp__neo__edit", "mcp__neo__write"]);
    expect(got.tools).toEqual(expect.arrayContaining(["tools_search", "read", "edit", "write"]));

    const read = String((await run(`neocall read {"path":"a.txt"}`)).text);
    const tag = /^(2:[^|]+)\|two$/m.exec(read)?.[1];
    if (tag === undefined) throw new Error(`no tag in ${read}`);
    const edits = [{ start: tag, newText: "TWO" }];
    const edit = await run(`neocall edit ${JSON.stringify({ path: "a.txt", edits })}`);
    expect(edit.isError).toBe(false);
    expect(readFileSync(join(dir, "a.txt"), "utf8")).toBe("one\nTWO\n");
    // A stale tag fails with the lines as they are now.
    const stale = await run(`neocall edit ${JSON.stringify({ path: "a.txt", edits })}`);
    expect(stale.isError).toBe(true);
  });

  test("a guest without a known adapter has no settings of neo's", async () => {
    const { daemon } = await setup({ adapter: undefined });
    const session = daemon.sessions.create({}).id;
    expect(await refused(daemon, session, "/settings fake-tools neo")).toBe(true);
  });
});

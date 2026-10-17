// Session titles written by a model: after the first message, again with /title, and on the model
// the user chose with /title-model.
import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import type { GenerateTextParams } from "@suite/harness/agent";
import type { SessionInfo } from "@suite/protocol/types";
import type { GuestConfig } from "../src/guests/guests.ts";
import { cleanTitle, DEFAULT_TITLE_MODEL, parseTitle, TITLE_ATTEMPTS } from "../src/titles.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** An agent that writes the titles it is told to: a string is a reply, an Error a failure. */
class TitleAgent extends FakeAgent {
  readonly asked: GenerateTextParams[] = [];
  constructor(private replies: (string | Error)[] = []) {
    super("main");
  }

  generateText = (params: GenerateTextParams): Promise<string> => {
    this.asked.push(params);
    const next = this.replies.shift();
    if (next === undefined) return Promise.reject(new Error("no reply scripted"));
    return next instanceof Error ? Promise.reject(next) : Promise.resolve(next);
  };
}

const FAKE: GuestConfig = {
  title: "Fake",
  command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
  permissions: "ask",
};

async function setup(
  options: { agent?: FakeAgent; guests?: Record<string, GuestConfig> } = {},
): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon({
    agent: options.agent ?? new TitleAgent(),
    titles: true,
    ...options,
  });
  cleanups.push(() => daemon.stop());
  return { daemon, session: daemon.sessions.create({}).id };
}

/** Waits until the session's info says so. */
async function until(
  daemon: TestDaemon,
  session: string,
  done: (info: SessionInfo) => boolean,
): Promise<SessionInfo> {
  for (let tries = 0; tries < 300; tries++) {
    const info = daemon.sessions.info(session);
    if (info && done(info)) return info;
    await Bun.sleep(10);
  }
  throw new Error(`the session never got there: ${JSON.stringify(daemon.sessions.info(session))}`);
}

/** Why a command failed; nothing when it did not. */
async function failure(work: Promise<void> | undefined): Promise<string> {
  try {
    await work;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return "";
}

const say = (daemon: TestDaemon, session: string, text: string): void => {
  daemon.store.append(session, { type: "user_message", app: "test", text });
};

describe("reading a title", () => {
  test("takes the title of the JSON it asked for, also inside fences or after a preamble", () => {
    expect(parseTitle('{"title": "Fix header scrollbar gutter"}')).toBe(
      "Fix header scrollbar gutter",
    );
    expect(parseTitle('```json\n{"title":"Dark mode for the header"}\n```')).toBe(
      "Dark mode for the header",
    );
    expect(parseTitle('Here it is: {"title": "Sticky table headers"} Hope it helps')).toBe(
      "Sticky table headers",
    );
  });

  test("a lone short line is a title too; anything else, or a JSON without one, is not", () => {
    expect(parseTitle("Sticky table headers")).toBe("Sticky table headers");
    expect(parseTitle("A title\nand something more")).toBeUndefined();
    expect(parseTitle('{"name": "x"}')).toBeUndefined();
    expect(parseTitle('{"title": 3}')).toBeUndefined();
    expect(parseTitle("")).toBeUndefined();
  });

  test("quotes and closing punctuation go, and so do titles that say nothing", () => {
    expect(cleanTitle('  "Fix   the header." ')).toBe("Fix the header");
    expect(cleanTitle("`Add search`:")).toBe("Add search");
    expect(cleanTitle("New thread")).toBeUndefined();
    expect(cleanTitle("new session")).toBeUndefined();
    expect(cleanTitle("Untitled")).toBeUndefined();
    expect(cleanTitle("   ")).toBeUndefined();
  });

  test("a long one is cut at a word", () => {
    const long = "Rework how the sidebar groups sessions by project and worktree in every view";
    const title = cleanTitle(long) ?? "";
    expect(title.length).toBeLessThanOrEqual(60);
    expect(title.endsWith("…")).toBe(true);
    expect(long.startsWith(title.slice(0, -1))).toBe(true);
    expect(title.slice(0, -1).endsWith("project and")).toBe(true);
  });
});

describe("the first message", () => {
  test("names the session by its start, then a model writes the title", async () => {
    const agent = new TitleAgent(['{"title": "Header scrollbar gutter"}']);
    const { daemon, session } = await setup({ agent });
    const changes: SessionInfo[] = [];
    daemon.sessions.onStateChange((id) => {
      const info = daemon.sessions.info(id);
      if (info) changes.push(info);
    });
    say(daemon, session, "the header jumps when the scrollbar appears, fix it please");
    // The start of the message right away, so a slow model never leaves a session untitled.
    expect(daemon.sessions.info(session)?.title).toBe(
      "the header jumps when the scrollbar appears, fix it please",
    );
    const info = await until(daemon, session, (i) => i.title === "Header scrollbar gutter");
    expect(info.title).toBe("Header scrollbar gutter");
    expect(daemon.store.getSession(session)?.titleSource).toBe("generated");
    // The clients hear of it (session.changed follows these).
    expect(changes.at(-1)?.title).toBe("Header scrollbar gutter");
    // The model is asked once, for the title, with the message and no more.
    expect(agent.asked).toHaveLength(1);
    expect(agent.asked[0]?.system).toContain('{"title"');
    expect(agent.asked[0]?.prompt).toContain("the header jumps when the scrollbar appears");
    expect(agent.asked[0]?.model).toBeUndefined();
  });

  test("only the first message asks", async () => {
    const agent = new TitleAgent(['{"title": "First"}', '{"title": "Second"}']);
    const { daemon, session } = await setup({ agent });
    say(daemon, session, "one");
    await until(daemon, session, (i) => i.title === "First");
    say(daemon, session, "two");
    await Bun.sleep(50);
    expect(daemon.sessions.info(session)?.title).toBe("First");
    expect(agent.asked).toHaveLength(1);
  });

  test("a title the user gave stays", async () => {
    const agent = new TitleAgent(['{"title": "Not this"}']);
    const { daemon } = await setup({ agent });
    const given = daemon.sessions.create({ title: "My own title" }).id;
    say(daemon, given, "something else");
    await Bun.sleep(50);
    expect(daemon.sessions.info(given)?.title).toBe("My own title");
    expect(agent.asked).toHaveLength(0);
  });

  test("tries again when the reply has no title, and when the call fails", async () => {
    const agent = new TitleAgent([
      new Error("overloaded"),
      "I cannot do that.\nSorry",
      '{"title": "Third time"}',
    ]);
    const { daemon, session } = await setup({ agent });
    say(daemon, session, "start");
    await until(daemon, session, (i) => i.title === "Third time");
    expect(agent.asked).toHaveLength(3);
  });

  test("keeps the start of the message when no attempt works, and says why on /title", async () => {
    const failing = Array.from({ length: TITLE_ATTEMPTS * 2 }, () => new Error("no credit left"));
    const agent = new TitleAgent(failing);
    const { daemon, session } = await setup({ agent });
    say(daemon, session, "fix the footer");
    await until(daemon, session, () => agent.asked.length === TITLE_ATTEMPTS);
    await Bun.sleep(30);
    expect(daemon.sessions.info(session)?.title).toBe("fix the footer");
    expect(daemon.store.getSession(session)?.titleSource).toBe("first_line");
    expect(await failure(daemon.commands.run(session, "/title"))).toContain(
      "main did not write a title: no credit left",
    );
  });

  test("a model that says New thread has not written a title", async () => {
    const agent = new TitleAgent(['{"title": "New thread"}', '{"title": "Footer links"}']);
    const { daemon, session } = await setup({ agent });
    say(daemon, session, "the links in the footer");
    await until(daemon, session, (i) => i.title === "Footer links");
    expect(agent.asked).toHaveLength(2);
  });

  test("an agent that can't write text leaves the start of the message", async () => {
    const { daemon, session } = await setup({ agent: new FakeAgent("main") });
    say(daemon, session, "hello there");
    await Bun.sleep(30);
    expect(daemon.sessions.info(session)?.title).toBe("hello there");
    expect(await failure(daemon.commands.run(session, "/title"))).toContain("can't write titles");
  });
});

describe("/title", () => {
  test("writes the title again from the conversation, which includes the old title", async () => {
    const agent = new TitleAgent(['{"title": "Old"}', '{"title": "Footer and header polish"}']);
    const { daemon, session } = await setup({ agent });
    say(daemon, session, "fix the header");
    await until(daemon, session, (i) => i.title === "Old");
    daemon.store.append(session, { type: "agent_message_chunk", messageId: "a", text: "Done." });
    say(daemon, session, "now the footer too");
    await daemon.commands.run(session, "/title");
    expect(daemon.sessions.info(session)?.title).toBe("Footer and header polish");
    const asked = agent.asked.at(-1);
    expect(asked?.system).toContain('The previous title was "Old"');
    expect(asked?.prompt).toContain("User: fix the header\nAgent: Done.");
    expect(asked?.prompt).toContain("User: now the footer too");
  });

  test("replaces a title the user gave, because the user asked", async () => {
    const agent = new TitleAgent(['{"title": "Generated"}']);
    const { daemon } = await setup({ agent });
    const given = daemon.sessions.create({ title: "Mine" }).id;
    say(daemon, given, "something");
    await daemon.commands.run(given, "/title");
    expect(daemon.sessions.info(given)?.title).toBe("Generated");
  });

  test("has nothing to write from before the first message, and takes no arguments", async () => {
    const { daemon, session } = await setup();
    expect(await failure(daemon.commands.run(session, "/title"))).toContain("nothing to title yet");
    expect(await failure(daemon.commands.run(session, "/title now"))).toContain("usage: /title");
  });
});

describe("the title model", () => {
  test("is the suite's choice until the user chooses: Claude's Haiku if there is such a guest", async () => {
    const withClaude = await setup({ guests: { claude: { ...FAKE, title: "Claude Code" } } });
    expect(withClaude.daemon.titles.current()).toEqual({
      model: DEFAULT_TITLE_MODEL,
      chosen: false,
    });
    const without = await setup();
    expect(without.daemon.titles.current()).toEqual({ model: { harness: "main" }, chosen: false });
  });

  test("/title-model sets it for every session, logs it, and goes back with default", async () => {
    const { daemon, session } = await setup({ guests: { fake: FAKE } });
    await daemon.commands.run(session, "/title-model fake large");
    expect(daemon.titles.current()).toEqual({
      model: { harness: "fake", model: "large" },
      chosen: true,
    });
    expect(daemon.store.read(session).map((r) => r.event)).toContainEqual({
      type: "setting_changed",
      harness: "suite",
      option: "title_model",
      value: "fake large",
    });
    // Another session, and a new daemon on the same data, have it too.
    const other = daemon.sessions.create({}).id;
    expect(daemon.settings.titleModel()).toEqual({ harness: "fake", model: "large" });
    expect(daemon.store.getMeta("setting:suite:title_model")).toBe("fake large");
    await daemon.commands.run(other, "/title-model default");
    expect(daemon.titles.current().chosen).toBe(false);
  });

  test("/title-model refuses a harness that doesn't exist and a model a guest doesn't list", async () => {
    const { daemon, session } = await setup({ guests: { fake: FAKE } });
    expect(await failure(daemon.commands.run(session, "/title-model nobody x"))).toContain(
      "usage:",
    );
    expect(await failure(daemon.commands.run(session, "/title-model default x"))).toContain(
      "usage:",
    );
    expect(await failure(daemon.commands.run(session, "/title-model main a b"))).toContain(
      "usage:",
    );
    // Asking for its models starts the guest's adapter, so they are known after that.
    await daemon.commands.settings(session);
    expect(await failure(daemon.commands.run(session, "/title-model fake huge"))).toContain(
      "Fake has no model huge: small, large",
    );
    expect(daemon.titles.current().chosen).toBe(false);
  });

  test("/title-model with nothing opens a choice of harness, and then of its model", async () => {
    const { daemon, session } = await setup({ guests: { fake: FAKE } });
    await daemon.commands.run(session, "/title-model");
    const asked = (): { id: string; title: string; options: { id: string }[] } => {
      const events = daemon.store.read(session).map((r) => r.event);
      const choice = events.filter((e) => e.type === "choice").at(-1);
      if (choice?.type !== "choice") throw new Error("no choice");
      return choice;
    };
    const answer = async (option: string): Promise<void> => {
      const { id } = asked();
      await daemon.actions.run(
        { session, ref: "c", action: "suite.choose", args: { id, option } },
        "test",
      );
    };
    expect(asked().title).toBe("Title model (now: main, the default)");
    expect(asked().options.map((o) => o.id)).toEqual(["default", "main", "fake"]);
    await answer("fake");
    await until(daemon, session, () => asked().title === "Fake model for titles");
    expect(asked().options.map((o) => o.id)).toEqual(["small", "large"]);
    await answer("small");
    await until(daemon, session, () => daemon.titles.current().chosen);
    expect(daemon.titles.current().model).toEqual({ harness: "fake", model: "small" });
  });

  test("a guest writes the title on the model chosen, in an adapter of its own", async () => {
    const { daemon, session } = await setup({ guests: { fake: FAKE } });
    await daemon.commands.run(session, "/title-model fake large");
    say(daemon, session, "fix the header\ntitle Header fix");
    // The adapter reports the model it was set to; nothing of this is in the session's log.
    await until(daemon, session, (i) => i.title === "Header fix (large)");
    const events = daemon.store.read(session).map((r) => r.event.type);
    expect(events).not.toContain("agent_message_chunk");
    expect(events).not.toContain("harness_message");
    expect(await daemon.guests.knownOptions(session, "fake")).toBeUndefined();
  });
});

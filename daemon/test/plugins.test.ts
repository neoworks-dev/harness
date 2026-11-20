import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { BadRequestError } from "../src/focus.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const MARKETPLACE = join(import.meta.dir, "..", "..", "neo", "test", "fixtures", "marketplace");

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function setup(): Promise<{
  daemon: TestDaemon;
  agent: FakeAgent;
  session: string;
  plugins: string;
}> {
  const plugins = mkdtempSync(join(tmpdir(), "neo-plugins-"));
  cleanups.push(() => rmSync(plugins, { recursive: true, force: true }));
  const agent = new FakeAgent("neo");
  const guests = {
    claude: { title: "Claude Code", command: ["false"], permissions: "allow" as const },
  };
  const daemon = await startTestDaemon({ agent, plugins, guests });
  cleanups.push(() => daemon.stop());
  return { daemon, agent, session: daemon.sessions.create({}).id, plugins };
}

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

function notices(daemon: TestDaemon, session: string): string[] {
  const out: string[] = [];
  for (const event of events(daemon, session)) if (event.type === "notice") out.push(event.text);
  return out;
}

function offered(daemon: TestDaemon, session: string, harness = "neo"): string[] | undefined {
  const found = events(daemon, session).findLast(
    (event) => event.type === "commands_available" && event.harness === harness,
  );
  if (found?.type !== "commands_available") return undefined;
  return found.commands.map((command) => command.name);
}

/** What a client typing in the session sends. */
function type(daemon: TestDaemon, session: string, text: string): void {
  const handle = daemon.hub.connect({ send: () => {} });
  handle.receive(JSON.stringify({ topic: "bus.hello", payload: { protocol: 1, client: "test" } }));
  handle.receive(
    JSON.stringify({ topic: "session.input", id: "i", payload: { session, app: "t", text } }),
  );
}

test("/plugin adds a marketplace, installs, lists, disables and removes", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, "/plugin list");
  expect(notices(daemon, session).at(-1)).toContain("No plugins installed.");
  expect(notices(daemon, session).at(-1)).toContain("/plugin marketplace add");

  await daemon.commands.run(session, `/plugin marketplace add ${MARKETPLACE}`);
  expect(notices(daemon, session).at(-1)).toContain(
    "Added marketplace mini-official with 4 plugins",
  );

  await daemon.commands.run(session, "/plugin search design");
  expect(notices(daemon, session).at(-1)).toContain("frontend-design@mini-official");

  await daemon.commands.run(session, "/plugin install frontend-design@mini-official");
  const installed = notices(daemon, session).at(-1) ?? "";
  expect(installed).toContain("Installed frontend-design@mini-official");
  expect(installed).toContain("1 skill");
  expect(installed).toContain("new sessions");

  await daemon.commands.run(session, "/plugin install commit-commands");
  expect(notices(daemon, session).at(-1)).toContain("2 commands");

  await daemon.commands.run(session, "/plugin list");
  const listing = notices(daemon, session).at(-1) ?? "";
  expect(listing).toContain("frontend-design@mini-official");
  expect(listing).toContain("(enabled)");
  expect(listing).toContain("mini-official (4 plugins)");

  await daemon.commands.run(session, "/plugin disable frontend-design");
  await daemon.commands.run(session, "/plugin list");
  expect(notices(daemon, session).at(-1)).toContain("frontend-design@mini-official (disabled)");
  await daemon.commands.run(session, "/plugin enable frontend-design");
  await daemon.commands.run(session, "/plugin uninstall frontend-design");
  await daemon.commands.run(session, "/plugin marketplace remove mini-official");
  expect(notices(daemon, session).at(-1)).toContain("uninstalled commit-commands@mini-official");
  expect(offered(daemon, session)).toEqual([]);
});

test("a mistake is a bad request that says what to type", async () => {
  const { daemon, session } = await setup();
  for (const text of [
    "/plugin nope",
    "/plugin install",
    "/plugin install a@b",
    "/plugin enable nothing",
    "/plugin marketplace add",
    `/plugin marketplace add ${join(tmpdir(), "no-such-marketplace-dir")}`,
  ]) {
    let error: unknown;
    try {
      await daemon.commands.run(session, text);
    } catch (caught) {
      error = caught;
    }
    expect(error).toBeInstanceOf(BadRequestError);
  }
});

test("the session is told neo's commands, and a typed /command reaches neo expanded", async () => {
  const { daemon, agent, session } = await setup();
  await daemon.commands.run(session, `/plugin marketplace add ${MARKETPLACE}`);
  await daemon.commands.run(session, "/plugin install commit-commands");
  const list = events(daemon, session).findLast((event) => event.type === "commands_available");
  expect(list).toMatchObject({
    harness: "neo",
    commands: [
      { name: "commit", description: "Create a git commit" },
      { name: "commit-push-pr", description: "Commit, push, and open a PR" },
    ],
  });

  type(daemon, session, "/commit fix the build");
  await agent.started(1);
  agent.release();
  await daemon.sessions.idle(session);
  const message = events(daemon, session).findLast((event) => event.type === "user_message");
  if (message?.type !== "user_message") throw new Error("no user message");
  expect(message.shown).toBe("/commit fix the build");
  expect(message.text).toContain("create a single git commit");
  expect(message.text).toContain("ARGUMENTS: fix the build");
  expect(agent.prompts[0]?.text).toContain("ARGUMENTS: fix the build");

  // Not a plugin command: sent as typed.
  type(daemon, session, "/unknown thing");
  await agent.started(2);
  agent.release();
  await daemon.sessions.idle(session);
  const plain = events(daemon, session).findLast((event) => event.type === "user_message");
  expect(plain).toMatchObject({ text: "/unknown thing" });
  expect(plain && "shown" in plain).toBe(false);
});

test("the daemon's own commands win a name, and the plugin's is reached as plugin:name", async () => {
  const { daemon, agent, session, plugins } = await setup();
  const root = join(plugins, "cache", "m", "p", "1");
  mkdirSync(join(root, "commands"), { recursive: true });
  writeFileSync(join(root, "commands", "model.md"), "Not the model setting");
  writeFileSync(join(root, "commands", "zap.md"), "Zap $ARGUMENTS");
  writeFileSync(
    join(plugins, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: { "p@m": { root, version: "1", enabled: true, installedAt: "" } },
    }),
  );
  // Told when the session next hears from the user (or when plugins change).
  type(daemon, session, "/zap now");
  await agent.started(1);
  agent.release();
  await daemon.sessions.idle(session);
  expect(offered(daemon, session)).toEqual(["p:model", "zap"]);
  expect(agent.prompts[0]?.text).toContain("Zap now");

  type(daemon, session, "/p:model please");
  await agent.started(2);
  agent.release();
  await daemon.sessions.idle(session);
  expect(agent.prompts[1]?.text).toContain("Not the model setting");
});

test("a session on another harness gets /commands as typed", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, `/plugin marketplace add ${MARKETPLACE}`);
  await daemon.commands.run(session, "/plugin install commit-commands");
  const other = daemon.sessions.create({}).id;
  daemon.settings.setHarness(other, "claude");
  type(daemon, other, "/commit now");
  await daemon.sessions.idle(other);
  const message = events(daemon, other).find((event) => event.type === "user_message");
  expect(message).toMatchObject({ text: "/commit now" });
  expect(message && "shown" in message).toBe(false);
});

test("/plugin alone is a menu that stays: add a marketplace, install from it, disable", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, "/plugin");
  const latest = () => {
    const choices = events(daemon, session).filter((e) => e.type === "choice");
    const last = choices.at(-1);
    if (last?.type !== "choice") throw new Error("no menu");
    return last;
  };
  // The menu comes once its data is read.
  for (let i = 0; i < 50 && !events(daemon, session).some((e) => e.type === "choice"); i++) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  const id = latest().id;
  const choose = (option: string, value?: string) =>
    daemon.actions.run(
      {
        session,
        ref: `q-${id}`,
        action: "neo.choose",
        args: { id, option, stay: true, ...(value === undefined ? {} : { value }) },
      },
      "test",
    );
  const settled = async (done: (title: string) => boolean): Promise<void> => {
    for (let i = 0; i < 100 && !done(latest().title); i++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
  };
  expect(latest().options.map((o) => o.id)).toEqual(["install", "add"]);

  await choose("add", MARKETPLACE);
  await settled((title) => title.includes("done"));
  // Added: on to what it offers.
  expect(latest().title).toContain("Install a plugin");
  expect(latest().options.map((o) => o.id)).toContain("get:frontend-design@mini-official");

  await choose("get:frontend-design@mini-official");
  await settled((title) => title.startsWith("Plugins") && title.includes("done"));
  expect(latest().options.map((o) => o.name)).toContain("frontend-design@mini-official · enabled");

  await choose("plugin:frontend-design@mini-official");
  await choose("toggle");
  await settled((title) => title.includes("done"));
  await choose("back");
  expect(latest().options.map((o) => o.name)).toContain("frontend-design@mini-official · disabled");
});

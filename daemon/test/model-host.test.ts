// /model on neo with OpenRouter: after a model without a host, a second card asks which host
// runs it. The harness is the real neo on a fake model and a fake fetch (no network).
import { afterEach, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { createNeoAgent } from "@neoworks-dev/neo/neo";
import type { SessionEvent } from "@suite/protocol/types";
import { fakeModel, TEST_CONFIG } from "../../harness/test/fake-model.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const fixture = async (name: string): Promise<unknown> =>
  JSON.parse(await readFile(join(import.meta.dir, "../../harness/test/fixtures", name), "utf8"));
const MODELS = await fixture("openrouter-models.json");
const ENDPOINTS = await fixture("openrouter-endpoints.json");

const NEW = "openrouter/deepseek/deepseek-v4.1-flash-1017";
const CONFIG = { ...TEST_CONFIG, provider: "openrouter", modelId: "anthropic/claude-haiku-4.5" };

async function setup(offline = false): Promise<{ daemon: TestDaemon; session: string }> {
  const agent = createNeoAgent({
    config: CONFIG,
    streamFn: fakeModel([]).streamFn,
    fetch: (url) => {
      if (offline) return Promise.reject(new Error("offline"));
      const body = url.endsWith("/endpoints") ? ENDPOINTS : MODELS;
      return Promise.resolve(Response.json(body));
    },
  });
  const daemon = await startTestDaemon({ agent });
  cleanups.push(() => daemon.stop());
  const session = daemon.sessions.create({}).id;
  // The live list is fetched in the background once the session exists: wait for it.
  const live = async (): Promise<boolean> =>
    (await daemon.sessions.configOptions(session)).some((o) =>
      o.values.some((v) => v.value === NEW),
    );
  for (let i = 0; i < 200 && !offline && !(await live()); i++) await Bun.sleep(5);
  return { daemon, session };
}

const events = (daemon: TestDaemon, session: string): SessionEvent[] =>
  daemon.store.read(session).map((r) => r.event);

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Choice = Extract<SessionEvent, { type: "choice" }>;

/** The open choice card, once there is one. */
async function openChoice(daemon: TestDaemon, session: string): Promise<Choice> {
  const open = (): Choice | undefined => {
    const log = events(daemon, session);
    return log.findLast(
      (e): e is Choice =>
        e.type === "choice" && !log.some((m) => m.type === "choice_made" && m.id === e.id),
    );
  };
  await until(() => open() !== undefined, "a choice");
  const found = open();
  if (!found) throw new Error("no choice");
  return found;
}

async function pick(
  daemon: TestDaemon,
  session: string,
  choice: Choice,
  option: string,
): Promise<void> {
  await daemon.actions.run(
    { session, ref: "x", action: "suite.choose", args: { id: choice.id, option } },
    "test",
  );
}

/** The values /model set, in order. */
function models(daemon: TestDaemon, session: string): string[] {
  const values: string[] = [];
  for (const e of events(daemon, session)) {
    if (e.type === "setting_changed" && e.option === "model") values.push(e.value);
  }
  return values;
}

test("a model typed without a host asks for one; the pick is set with its @tag", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, `/model ${NEW}`);
  const card = await openChoice(daemon, session);
  expect(card.title).toBe("Host for DeepSeek: DeepSeek V4.1 Flash 1017");
  expect(card.options[0]).toEqual({ id: NEW, name: "Any (OpenRouter chooses)" });
  expect(card.options.map((o) => o.id)).toContain(`${NEW}@deepinfra/fp8`);
  expect(models(daemon, session)).toEqual([NEW]);

  await pick(daemon, session, card, `${NEW}@deepinfra/fp8`);
  await until(() => models(daemon, session).length === 2, "the host");
  expect(models(daemon, session)).toEqual([NEW, `${NEW}@deepinfra/fp8`]);
  expect(daemon.settings.get(session, "neo")).toEqual({ model: `${NEW}@deepinfra/fp8` });
  const options = await daemon.sessions.configOptions(session);
  expect(options.find((o) => o.category === "model")?.current).toBe(`${NEW}@deepinfra/fp8`);
  // The answer does not ask again.
  await Bun.sleep(50);
  expect(events(daemon, session).filter((e) => e.type === "choice")).toHaveLength(1);
});

test("'Any' leaves the model without a host", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, `/model ${NEW}@wafer/mxfp4`);
  expect(models(daemon, session)).toEqual([`${NEW}@wafer/mxfp4`]);
  // Typed with a host: nothing to ask.
  await Bun.sleep(50);
  expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);

  await daemon.commands.run(session, `/model ${NEW}`);
  const card = await openChoice(daemon, session);
  await pick(daemon, session, card, NEW);
  await until(() => models(daemon, session).length === 3, "the answer");
  expect(models(daemon, session).at(-1)).toBe(NEW);
  expect(daemon.settings.get(session, "neo")).toEqual({ model: NEW });
});

test("a model picked from the list asks for its host as well", async () => {
  const { daemon, session } = await setup();
  await daemon.commands.run(session, "/model");
  const list = await openChoice(daemon, session);
  expect(list.options.map((o) => o.id)).toContain(NEW);
  await pick(daemon, session, list, NEW);
  await until(
    () => events(daemon, session).filter((e) => e.type === "choice").length === 2,
    "host",
  );
  const card = events(daemon, session).findLast((e): e is Choice => e.type === "choice");
  expect(card?.title).toStartWith("Host for ");
});

test("when the hosts can't be listed the step is skipped with a notice", async () => {
  const { daemon, session } = await setup(true);
  await daemon.commands.run(session, `/model ${NEW}`);
  await until(() => events(daemon, session).some((e) => e.type === "notice"), "the notice");
  const notice = events(daemon, session).find((e) => e.type === "notice");
  expect(notice).toMatchObject({ type: "notice", tone: "muted" });
  expect(JSON.stringify(notice)).toContain("OpenRouter chooses");
  expect(events(daemon, session).some((e) => e.type === "choice")).toBe(false);
  expect(models(daemon, session)).toEqual([NEW]);
});

test("a host on a model of another provider is refused", async () => {
  const { daemon, session } = await setup();
  expect(
    daemon.commands.run(session, "/model deepseek/deepseek-v4-pro@deepinfra/fp8"),
  ).rejects.toThrow("not OpenRouter");
});

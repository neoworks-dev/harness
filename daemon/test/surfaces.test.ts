import { afterEach, describe, expect, test } from "bun:test";
import type {
  SessionEvent,
  SurfaceOpened,
  SurfacePlace,
  UiDoc,
  ViewItem,
  ViewPatch,
} from "@neoworks-dev/neo-protocol/types";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { silentLogger } from "../src/log.ts";
import { parseSurfaceFile, surfaceMenu } from "../src/surfaces.ts";
import { mistakesIn } from "../src/ui-lint.ts";
import { startTestDaemon } from "../src/testing.ts";
import { SessionView } from "../src/view/session-view.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const opened = (name: string, source: string, place: SurfacePlace = "bottom"): SurfaceOpened => ({
  type: "surface_opened",
  name,
  place,
  source,
});

function view(events: SessionEvent[]): SessionView {
  const v = new SessionView("s", () => undefined, silentLogger);
  events.forEach((event, i) => v.apply({ session: "s", seq: i + 1, ts: 0, event }));
  return v;
}

const ui = (item: ViewItem | undefined): UiDoc | undefined =>
  item?.component.type === "neoui" ? item.component.ui : undefined;

describe("surfaces in the view", () => {
  test("an open surface is drawn apart from the chat, in its place; a tail never cuts it", () => {
    const v = view([
      { type: "notice", text: "one" },
      opened("bar", '<Progress value={0.5} label="half"/>'),
      { type: "notice", text: "two" },
    ]);
    const snapshot = v.snapshot(1);
    expect(snapshot.items.map((item) => item.ref)).toEqual(["n3"]);
    expect(snapshot.surfaces).toHaveLength(1);
    const surface = snapshot.surfaces?.[0];
    expect(surface).toMatchObject({ ref: "s-bar", place: "bottom", label: "bar" });
    expect(ui(surface)?.root).toMatchObject({ component: "Progress", props: { value: 0.5 } });
  });

  test("opening a name again replaces it; closing removes it", () => {
    const v = new SessionView("s", () => undefined, silentLogger);
    const first = v.apply({ session: "s", seq: 1, ts: 0, event: opened("bar", "<Text>a</Text>") });
    // A surface is not a chat item: its set op is never `appended`.
    expect(first).toEqual([expect.objectContaining({ op: "set" })]);
    expect(first[0]).not.toHaveProperty("appended");
    v.apply({
      session: "s",
      seq: 2,
      ts: 0,
      event: { ...opened("bar", "<Text>b</Text>", "side"), title: "Bar" },
    });
    const surfaces = v.snapshot().surfaces ?? [];
    expect(surfaces).toHaveLength(1);
    expect(surfaces[0]).toMatchObject({ place: "side", label: "Bar" });
    expect(JSON.stringify(ui(surfaces[0]))).toContain('"b"');
    const closed = v.apply({
      session: "s",
      seq: 3,
      ts: 0,
      event: { type: "surface_closed", name: "bar" },
    });
    expect(closed).toEqual([{ op: "remove", ref: "s-bar" }]);
    expect(v.snapshot().surfaces).toBeUndefined();
    // Closing what is not open changes nothing.
    expect(
      v.apply({ session: "s", seq: 4, ts: 0, event: { type: "surface_closed", name: "bar" } }),
    ).toEqual([]);
  });

  test("a replayed log opens what was left open", () => {
    const v = view([
      opened("a", "<Text>a</Text>"),
      opened("b", "<Text>b</Text>"),
      { type: "surface_closed", name: "a" },
    ]);
    expect(v.snapshot().surfaces?.map((item) => item.ref)).toEqual(["s-b"]);
  });

  test("a surface's query polls only while the view is watched, and stops when it closes", async () => {
    let calls = 0;
    let watched = true;
    const v = new SessionView(
      "s",
      () => undefined,
      silentLogger,
      undefined,
      () => Promise.resolve({ n: ++calls }),
      undefined,
      () => watched,
    );
    v.apply({
      session: "s",
      seq: 1,
      ts: 0,
      event: opened(
        "p",
        '<Query id="q" hx-get="x.n" hx-trigger="load, every 1s" default={{ n: 0 }}/><Text>{q.n}</Text>',
      ),
    });
    const wait = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 1100));
    await wait();
    expect(calls).toBeGreaterThan(1);
    expect(JSON.stringify(v.snapshot().surfaces)).toContain(String(calls));
    watched = false;
    const unwatched = calls;
    await wait();
    expect(calls).toBe(unwatched);
    watched = true;
    v.apply({ session: "s", seq: 2, ts: 0, event: { type: "surface_closed", name: "p" } });
    const closed = calls;
    await wait();
    expect(calls).toBe(closed);
    v.dispose();
  });
});

describe("using a surface", () => {
  test("typing into a surface's field patches it; its button asks the agent", async () => {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const session = daemon.focus.current().session;
    const patches: ViewPatch[] = [];
    daemon.views.get(session);
    daemon.views.onPatch((patch) => patches.push(patch));
    daemon.store.append(
      session,
      opened(
        "knobs",
        '<Form name="vars"><Input name="lr" value="0.1"/><Buttons><Button id="set" ui-ask="Use these">Set</Button></Buttons></Form>',
        "page",
      ),
    );
    const act = (action: string, args: unknown): Promise<void> =>
      daemon.actions.run({ session, ref: "s-knobs", action, args }, "test");

    await act("neoui.input", { doc: "0", key: "lr", value: "0.01" });
    const set = patches.at(-1)?.ops[0];
    expect(set).toMatchObject({ op: "set", item: { ref: "s-knobs", place: "page" } });
    expect(JSON.stringify(set)).toContain('"value":"0.01"');

    await act("neoui.press", { doc: "0", key: "set" });
    await daemon.sessions.idle(session);
    const sent = daemon.store.readTypes(session, ["user_message"]).at(-1)?.event;
    expect(sent).toMatchObject({ type: "user_message" });
    expect(JSON.stringify(sent)).toContain('{\\"lr\\":\\"0.01\\"}');
  });
});

describe("the ui_surface tool", () => {
  const said = (result: { content: { type: string; text?: string }[] }): string =>
    result.content.map((part) => part.text ?? "").join("\n");

  test("open, update, list and close, through the gateway; the view follows", async () => {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const session = daemon.focus.current().session;
    const call = (args: Record<string, unknown>) =>
      daemon.gateway.call("ui_surface", args, undefined, { session });

    const first = await call({
      action: "open",
      name: "run",
      source: '<Progress value={0.2} label="run"/>',
      title: "Run",
    });
    expect(said(first)).toBe("Opened run (bottom).");
    const updated = await call({
      action: "open",
      name: "run",
      source: '<Progress value={0.6} label="run"/>',
    });
    expect(said(updated)).toBe("Updated run (bottom).");
    const surfaces = daemon.views.get(session).snapshot().surfaces ?? [];
    expect(surfaces).toHaveLength(1);
    // The update keeps the title and place it had.
    expect(surfaces[0]).toMatchObject({ ref: "s-run", label: "Run", place: "bottom" });
    expect(JSON.stringify(surfaces[0])).toContain("0.6");
    expect(said(await call({ action: "list" }))).toBe("Open:\n- run (bottom): Run");

    expect(said(await call({ action: "close", name: "run" }))).toBe("Closed run.");
    expect(daemon.views.get(session).snapshot().surfaces).toBeUndefined();
    expect(said(await call({}))).toBe("No surfaces are open.");
  });

  test("a program with mistakes, a bad name or place is refused, and nothing opens", async () => {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const session = daemon.focus.current().session;
    const call = (args: Record<string, unknown>) =>
      daemon.gateway.call("ui_surface", args, undefined, { session });

    const wrong = await call({ action: "open", name: "x", source: "<Progres value={1}/>" });
    expect(wrong.isError).toBe(true);
    expect(said(wrong)).toContain("Progress");
    expect(
      (await call({ action: "open", name: "Bad Name", source: "<Text>a</Text>" })).isError,
    ).toBe(true);
    expect(
      (await call({ action: "open", name: "x", source: "<Text>a</Text>", place: "top" })).isError,
    ).toBe(true);
    expect((await call({ action: "close", name: "x" })).isError).toBe(true);
    expect(daemon.store.readTypes(session, ["surface_opened"])).toHaveLength(0);
  });
});

describe("surface files", () => {
  test("a file's frontmatter names it and its place; without one, the file's name and the bottom", () => {
    expect(
      parseSurfaceFile(
        "---\nname: run\nplace: side\ntitle: Run\nopen: always\n---\n<Text>a</Text>\n",
        "f",
        "user",
      ),
    ).toEqual({
      name: "run",
      place: "side",
      title: "Run",
      always: true,
      source: "<Text>a</Text>",
      from: "user",
    });
    expect(parseSurfaceFile("<Text>a</Text>", "bar", "user")).toMatchObject({
      name: "bar",
      place: "bottom",
      always: false,
    });
    expect(parseSurfaceFile("---\nplace: top\n---\n<Text>a</Text>", "bar", "user")).toBeUndefined();
    expect(parseSurfaceFile("---\nname: x\n---\n", "bar", "user")).toBeUndefined();
  });

  test("/surface opens a file, lists, closes; the agent opens one by name; `open: always` opens with a session", async () => {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const dir = join(daemon.dir, "surfaces");
    mkdirSync(dir, { recursive: true });
    writeFileSync(
      join(dir, "knobs.neoui"),
      '---\nplace: page\ntitle: Knobs\n---\n<Slider name="lr" min={0} max={1} value={0.5}/>',
    );
    writeFileSync(join(dir, "clock.neoui"), "---\nopen: always\n---\n<Text>tick</Text>");
    writeFileSync(join(dir, "broken.neoui"), "<Nope/>");
    const session = daemon.focus.current().session;
    const input = (text: string) => daemon.commands.run(session, text);
    const notices = () =>
      daemon.store
        .readTypes(session, ["notice"])
        .map((r) => (r.event.type === "notice" ? r.event.text : ""));

    await input("/surface knobs");
    expect(daemon.views.get(session).snapshot().surfaces).toMatchObject([
      { ref: "s-knobs", place: "page", label: "Knobs" },
    ]);
    await input("/surface list");
    expect(notices().at(-1)).toContain("- knobs (page): Knobs");
    expect(notices().at(-1)).toContain("- clock (bottom, user)");
    await input("/surface broken");
    expect(notices().at(-1)).toContain("mistakes");
    await input("/surface close knobs");
    expect(daemon.views.get(session).snapshot().surfaces).toBeUndefined();

    const opened = await daemon.gateway.call(
      "ui_surface",
      { action: "open", name: "knobs" },
      undefined,
      { session },
    );
    expect(opened.isError).toBeUndefined();
    expect(daemon.views.get(session).snapshot().surfaces?.[0]).toMatchObject({ place: "page" });

    const fresh = daemon.sessions.create({}).id;
    expect(daemon.views.get(fresh).snapshot().surfaces).toMatchObject([
      { ref: "s-clock", place: "bottom" },
    ]);
  });
});

test("the training example's surface files have no mistakes", () => {
  const dir = join(import.meta.dir, "../../examples/training/surfaces");
  for (const name of ["training", "training-vars"]) {
    const file = parseSurfaceFile(
      readFileSync(join(dir, `${name}.neoui`), "utf8"),
      name,
      "training",
    );
    expect(file).toBeDefined();
    expect(mistakesIn(`\`\`\`neoui\n${file?.source}\n\`\`\``)).toEqual([]);
  }
});

test("a surface's close button (neo.surface.close on its ref) closes it", async () => {
  const daemon = await startTestDaemon();
  cleanups.push(() => daemon.stop());
  const session = daemon.focus.current().session;
  daemon.store.append(session, opened("bar", "<Text>a</Text>"));
  await daemon.actions.run({ session, ref: "s-bar", action: "neo.surface.close" }, "test");
  expect(daemon.views.get(session).snapshot().surfaces).toBeUndefined();
  await expect(
    daemon.actions.run({ session, ref: "s-bar", action: "neo.surface.close" }, "test"),
  ).rejects.toThrow("no surface bar is open");
});

test("the /surface menu lists the files and the agent's surfaces; a pick opens or closes one", () => {
  const files = [
    {
      name: "run",
      place: "bottom" as const,
      title: "Run",
      always: false,
      source: "<Text>a</Text>",
      from: "user",
    },
  ];
  const menu = surfaceMenu(files, [opened("mine", "<Text>b</Text>", "side")]);
  expect(menu.stays).toBe(true);
  expect(menu.options.map((o) => [o.id, o.name])).toEqual([
    ["run", "Run · closed"],
    ["mine", "mine · open"],
  ]);
  const openNow = surfaceMenu(files, [opened("run", "<Text>a</Text>")]);
  expect(openNow.options[0]?.name).toBe("Run · open");
});

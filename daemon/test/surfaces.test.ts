import { afterEach, describe, expect, test } from "bun:test";
import type {
  SessionEvent,
  SurfaceOpened,
  SurfacePlace,
  UiDoc,
  ViewItem,
  ViewPatch,
} from "@neoworks-dev/neo-protocol/types";
import { silentLogger } from "../src/log.ts";
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

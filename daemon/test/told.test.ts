import { expect, test } from "bun:test";
import type { AppConnected } from "@suite/protocol/types";
import type { ConnectorSnapshot } from "../src/gateway/registry.ts";
import { ToldState } from "../src/told.ts";

const photo = (extra: Partial<ConnectorSnapshot> = {}): ConnectorSnapshot => ({
  app: "photo-editor",
  kind: "app",
  v: 1,
  tools: ["crop"],
  description: "Edits images.",
  ...extra,
});

function told(
  state: ToldState,
  current: ConnectorSnapshot[],
  describe = true,
): Omit<AppConnected, "type">[] {
  const events = state.changes(current, undefined);
  for (const event of events) if (event.type === "app_connected") state.apply(event);
  return events.flatMap((event) => {
    if (event.type !== "app_connected") return [];
    const { type: _, ...rest } = event;
    if (!describe) delete rest.description;
    return [rest];
  });
}

test("the first announcement of an app names its components; a session that has them is not told again", () => {
  const state = new ToldState();
  const first = told(state, [
    photo({ components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"] }),
  ]);
  expect(first).toEqual([
    {
      app: "photo-editor",
      kind: "app",
      v: 1,
      tools: ["crop"],
      components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"],
      description: "Edits images.",
    },
  ]);
  expect(
    told(state, [photo({ components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"] })]),
  ).toEqual([]);
});

test("an app that closes and comes back unchanged says nothing of its components", () => {
  const state = new ToldState();
  told(state, [photo({ components: ["photo-editor.BrightnessCard"] })]);
  state.apply({ type: "app_disconnected", app: "photo-editor" });
  expect(told(state, [photo({ components: ["photo-editor.BrightnessCard"] })])).toEqual([
    { app: "photo-editor", kind: "app", v: 1, tools: ["crop"], reconnect: true },
  ]);
});

test("a new version lists the components again when they changed, and empty when none are left", () => {
  const state = new ToldState();
  told(state, [photo({ components: ["photo-editor.BrightnessCard"] })]);
  expect(
    told(
      state,
      [photo({ v: 2, components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"] })],
      false,
    ),
  ).toEqual([
    {
      app: "photo-editor",
      kind: "app",
      v: 2,
      tools: ["crop"],
      previous: 1,
      components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"],
    },
  ]);
  // The tools changed, the components did not: not repeated.
  expect(
    told(state, [
      photo({
        v: 3,
        tools: ["crop", "resize"],
        components: ["photo-editor.BrightnessCard", "photo-editor.PhotoCard"],
      }),
    ]),
  ).toEqual([
    {
      app: "photo-editor",
      kind: "app",
      v: 3,
      tools: ["crop", "resize"],
      previous: 2,
      added: ["resize"],
    },
  ]);
  expect(told(state, [photo({ v: 4, tools: ["crop", "resize"] })])).toEqual([
    {
      app: "photo-editor",
      kind: "app",
      v: 4,
      tools: ["crop", "resize"],
      previous: 3,
      components: [],
    },
  ]);
});

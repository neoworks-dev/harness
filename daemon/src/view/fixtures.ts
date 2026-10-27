// Fixture inputs → resolved view items. Shared by the generator script and the daemon's tests.
// Two kinds: a template + data (one item), or a list of session events (a whole chat, resolved
// the way the daemon resolves a live session, templates by tool name). neoui docs in either can
// then be used: `inputs` are neoui.input calls applied in order, `presses` neoui.press calls
// after them.
import type {
  Component,
  ItemKind,
  ItemState,
  SessionEvent,
  ViewItem,
} from "@neoworks-dev/neo-protocol/types";
import { silentLogger } from "../log.ts";
import { isNeoUiTemplate, NeoUiDocs, templateData } from "./neoui.ts";
import { SessionView } from "./session-view.ts";
import { resolveTemplate } from "./template.ts";

/** A neoui.input (with `value`) or neoui.press call on a doc of the fixture. */
export interface NeoUiCall {
  doc: string;
  key: string;
  value?: unknown;
}

interface Interaction {
  inputs?: NeoUiCall[];
  presses?: NeoUiCall[];
}

export type Fixture = { description: string } & Interaction &
  (
    | {
        item: { kind: ItemKind; label?: string; state?: ItemState };
        template: unknown;
        data: unknown;
      }
    | { events: SessionEvent[]; templates?: Record<string, unknown> }
  );

async function templateComponent(
  name: string,
  fixture: Extract<Fixture, { template: unknown }>,
): Promise<Component> {
  const { template, data } = fixture;
  if (!isNeoUiTemplate(template)) return resolveTemplate(template, data);
  const docs = new NeoUiDocs(() => undefined);
  const provided = templateData(data);
  docs.doc(name, "0", template.neoui, false, provided);
  await interact(fixture, (doc) => docs.program(name, doc));
  const ui = docs.doc(name, "0", template.neoui, false, provided);
  if (!ui.root) throw new Error(`${name}: the neoui template drew nothing`);
  return { type: "neoui", ui };
}

async function resolveTemplateFixture(
  name: string,
  fixture: Extract<Fixture, { template: unknown }>,
): Promise<ViewItem[]> {
  const item: ViewItem = {
    ref: name,
    kind: fixture.item.kind,
    component: await templateComponent(name, fixture),
  };
  if (fixture.item.label !== undefined) item.label = fixture.item.label;
  if (fixture.item.state !== undefined) item.state = fixture.item.state;
  return [item];
}

/** Applies the fixture's inputs, then its presses, to the programs `find` knows. */
async function interact(
  fixture: Interaction,
  find: (doc: string) => ReturnType<NeoUiDocs["program"]>,
): Promise<void> {
  for (const call of fixture.inputs ?? []) {
    const program = find(call.doc);
    if (!program?.input(call.key, call.value)) {
      throw new Error(`neoui.input ${call.doc}/${call.key} was refused`);
    }
  }
  for (const call of fixture.presses ?? []) {
    const program = find(call.doc);
    if (!program) throw new Error(`neoui.press: no doc ${call.doc}`);
    await program.press(call.key);
  }
}

async function resolveEventFixture(
  name: string,
  fixture: Extract<Fixture, { events: SessionEvent[] }>,
): Promise<ViewItem[]> {
  const templates = fixture.templates ?? {};
  const view = new SessionView(name, (tool) => templates[tool], silentLogger);
  fixture.events.forEach((event, i) => view.apply({ session: name, seq: i + 1, ts: 0, event }));
  const refs = view.snapshot().items.map((item) => item.ref);
  // The calls address a doc by id; the item is the first that has one.
  const find = (doc: string): ReturnType<NeoUiDocs["program"]> => {
    for (const ref of refs) {
      const program = view.program(ref, doc);
      if (program) return program;
    }
    return undefined;
  };
  await interact(fixture, find);
  for (const ref of refs) view.changed(ref);
  return view.snapshot().items;
}

export async function resolveFixture(name: string, fixture: Fixture): Promise<ViewItem[]> {
  if ("events" in fixture) return resolveEventFixture(name, fixture);
  return resolveTemplateFixture(name, fixture);
}

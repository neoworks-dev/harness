// Fixture inputs → resolved view items. Shared by the generator script and the daemon's tests.
// Two kinds: a template + data (one item), or a list of session events (a whole chat, resolved
// the way the daemon resolves a live session, templates by tool name).
import type { ItemKind, ItemState, SessionEvent, ViewItem } from "@suite/protocol/types";
import { silentLogger } from "../log.ts";
import { SessionView } from "./session-view.ts";
import { resolveTemplate } from "./template.ts";

export type Fixture =
  | {
      description: string;
      item: { kind: ItemKind; label?: string; state?: ItemState };
      template: unknown;
      data: unknown;
    }
  | { description: string; events: SessionEvent[]; templates?: Record<string, unknown> };

export function resolveFixture(name: string, fixture: Fixture): ViewItem[] {
  if ("events" in fixture) {
    const templates = fixture.templates ?? {};
    const view = new SessionView(name, (tool) => templates[tool], silentLogger);
    fixture.events.forEach((event, i) => view.apply({ session: name, seq: i + 1, ts: 0, event }));
    return view.snapshot().items;
  }
  const item: ViewItem = {
    ref: name,
    kind: fixture.item.kind,
    component: resolveTemplate(fixture.template, fixture.data),
  };
  if (fixture.item.label !== undefined) item.label = fixture.item.label;
  if (fixture.item.state !== undefined) item.state = fixture.item.state;
  return [item];
}

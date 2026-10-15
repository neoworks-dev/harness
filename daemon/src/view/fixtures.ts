// Fixture inputs → resolved view items. Shared by the generator script and the daemon's tests.
import type { ItemKind, ItemState, ViewItem } from "@suite/protocol/types";
import { resolveTemplate } from "./template.ts";

export interface Fixture {
  description: string;
  item: { kind: ItemKind; label?: string; state?: ItemState };
  template: unknown;
  data: unknown;
}

export function resolveFixture(name: string, fixture: Fixture): ViewItem[] {
  const item: ViewItem = {
    ref: name,
    kind: fixture.item.kind,
    component: resolveTemplate(fixture.template, fixture.data),
  };
  if (fixture.item.label !== undefined) item.label = fixture.item.label;
  if (fixture.item.state !== undefined) item.state = fixture.item.state;
  return [item];
}

// The `slots` service: what a plugin registers its panes with. Every call goes through
// `ctx.effect`, so unloading the plugin takes its entries out again. It holds no state of its
// own (see the README: no `$state` fields on a Service), only the registry it delegates to.
import { Service, type Context } from "@neoworks/extension-system";
import { slotRegistry } from "./instance.ts";
import type { SlotRegistry } from "./registry.ts";
import type { EntrySpec, SlotHandle, SlotName } from "./types.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    slots: SlotsService;
  }
}

export class SlotsService extends Service {
  readonly registry: SlotRegistry;

  constructor(ctx: Context, registry: SlotRegistry = slotRegistry) {
    super(ctx, "slots");
    this.registry = registry;
  }

  /** Registers into a slot that is declared now; the plugin fails to load if it is not. */
  add<N extends SlotName>(name: N, spec: EntrySpec<N>): () => void {
    const ctx = this.ctx;
    return ctx.effect(() => this.registry.add(name, { ...spec, ctx }), `slots.add(${name})`);
  }

  /**
   * Registers into a slot whenever its owner is mounted: `run` gets the slot then, and what it
   * adds goes with the slot. Use it for everything but the plugin that owns the slot's parent.
   */
  inject<N extends SlotName>(
    name: N,
    run: (slot: SlotHandle<N>) => (() => void) | void,
  ): () => void {
    const ctx = this.ctx;
    return ctx.effect(
      () =>
        this.registry.contribute(name, (slot) =>
          run({ add: (spec) => slot.add({ ...spec, ctx }) }),
        ),
      `slots.inject(${name})`,
    );
  }
}

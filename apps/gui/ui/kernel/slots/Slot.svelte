<script lang="ts" generics="N extends SlotName">
  // Draws what is registered into the slot `name`, and again whenever that changes. `props` go to
  // every occupant, `key` picks the renderer in a keyed slot. See the kernel README.
  import { slotRegistry } from "./instance.ts";
  import Occupant from "./Occupant.svelte";
  import type { SlotArgs, SlotName } from "./types.ts";

  const args: SlotArgs<N> = $props();

  // Subscribed once for the component, not from inside the derived: createSubscriber read there
  // tears its subscription down on every rerun, and the teardown bumps it, which reruns the
  // derived again, forever.
  let changes = $state(0);
  $effect.pre(() => slotRegistry.subscribe(() => changes++));
  const occupants = $derived.by(() => {
    void changes;
    const { name, key } = args as { name: string; key?: string };
    return slotRegistry.resolve(name, key, args.props);
  });
</script>

{#each occupants as occupant (occupant.id)}
  <Occupant {occupant} props={args.props} />
{/each}

<script lang="ts">
  // One entry of a slot, drawn in the context of the plugin that registered it.
  import { provideCtx } from "./context.ts";
  import type { Occupant } from "./registry.ts";

  let { occupant, props }: { occupant: Occupant; props?: Record<string, unknown> } = $props();

  // Fixed for the component: another entry is another `occupant`, drawn anew.
  // svelte-ignore state_referenced_locally
  const Drawn = occupant.component;
  // svelte-ignore state_referenced_locally
  provideCtx(occupant.ctx as Parameters<typeof provideCtx>[0]);
</script>

<Drawn {...props} />

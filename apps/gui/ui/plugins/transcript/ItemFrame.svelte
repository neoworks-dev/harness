<script lang="ts">
  // The room around one chat item, whatever it is.
  import type { Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { children }: { children: Snippet } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View style={{ paddingTop: p.space.m, paddingBottom: p.space.s }}>
  {@render children()}
</View>

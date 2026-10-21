<script lang="ts">
  // The agent's reply as plain prose, without a bubble, under the name of the guest it is from,
  // if any.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { PREVIEW_LINES, proseColor } from "./layout.ts";
  import { transcript } from "./state.svelte.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame gap={item.component.type === "text" ? 8 : 16}>
  <View style={{ paddingX: 4, gap: p.space.s, color: proseColor(p) }}>
    {#if item.label}
      <Text style={{ color: p.muted, fontSize: 12, fontWeight: "medium" }}>{item.label}</Text>
    {/if}
    {#if item.component.type === "text"}
      <Component component={item.component} ref={item.ref} />
    {:else}
      <Component component={item.component} ref={item.ref} limit={transcript.expanded ? undefined : PREVIEW_LINES} />
    {/if}
  </View>
</ItemFrame>

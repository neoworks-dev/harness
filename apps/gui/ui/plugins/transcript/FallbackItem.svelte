<script lang="ts">
  // An item of a kind no plugin draws: its component as it is.
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame>
  <View style={{ paddingX: p.space.l }}>
    <Component component={item.component} ref={item.ref} />
  </View>
</ItemFrame>

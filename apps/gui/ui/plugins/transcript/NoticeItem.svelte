<script lang="ts">
  // A notice as one muted line, red when it failed.
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER } from "./layout.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame>
  <View style={{ paddingX: p.space.l, paddingLeft: p.space.l + GUTTER, color: item.state === "failed" ? p.danger : p.muted }}>
    <Component component={item.component} ref={item.ref} />
  </View>
</ItemFrame>

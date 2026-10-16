<script lang="ts">
  // The user's message on a band, behind a ›. What other plugins add (the app it came from) goes
  // after the text, in `chat.item.header`.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER } from "./layout.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame>
  <View style={{ direction: "row", background: p.band, paddingX: p.space.l, paddingY: p.space.m }}>
    <Text style={{ width: GUTTER, color: p.accent, fontWeight: "bold" }}>›</Text>
    <View style={{ flex: 1, minWidth: 0 }}>
      <Component component={item.component} ref={item.ref} />
    </View>
    <Slot name="chat.item.header" props={{ item }} />
  </View>
</ItemFrame>

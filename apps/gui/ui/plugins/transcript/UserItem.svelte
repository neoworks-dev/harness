<script lang="ts">
  // The user's message in a bubble at the right. What other plugins add (the app it came from)
  // goes under it, in `chat.item.header`.
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame>
  <View style={{ gap: 4, paddingTop: 8 }}>
    <View style={{ direction: "row", justify: "end" }}>
      <View style={{ shrink: 1, minWidth: 0, maxWidth: "80%", background: p.message, radius: p.radius.l, padding: 12 }}>
        <Component component={item.component} ref={item.ref} />
      </View>
    </View>
    <View style={{ direction: "row", justify: "end", paddingRight: 4 }}>
      <Slot name="chat.item.header" props={{ item }} />
    </View>
  </View>
</ItemFrame>

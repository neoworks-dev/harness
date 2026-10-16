<script lang="ts">
  // The agent's thoughts, muted and italic, hanging from a brain.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import brain from "@phosphor-icons/core/regular/brain.svg";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER } from "./layout.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<ItemFrame gap={8}>
  <View style={{ direction: "row", color: p.muted, italic: true }}>
    <View style={{ width: GUTTER, height: 22, align: "center", justify: "center" }}>
      <Icon svg={brain} size={16} />
    </View>
    <View style={{ flex: 1, minWidth: 0 }}>
      <Component component={item.component} ref={item.ref} />
    </View>
  </View>
</ItemFrame>

<script lang="ts">
  // A notice as one muted line. A failure (the turn ended in an error) is a warning in red and
  // its message in the prose's colour.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import warning from "@phosphor-icons/core/regular/warning-circle.svg";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER, proseColor } from "./layout.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const failed = $derived(
    item.state === "failed" || (item.component.type === "text" && item.component.tone === "danger"),
  );
</script>

<ItemFrame gap={6}>
  {#if failed}
    <View style={{ direction: "row", gap: 4, fontSize: 14, color: proseColor(p) }}>
      <View style={{ width: GUTTER, height: 24, align: "center", justify: "center", color: p.danger }}>
        <Icon svg={warning} size={16} />
      </View>
      <View style={{ flex: 1, minWidth: 0, paddingTop: 1 }}>
        <Component component={item.component} ref={item.ref} />
      </View>
    </View>
  {:else}
    <View style={{ paddingLeft: GUTTER, fontSize: 13, color: p.muted }}>
      <Component component={item.component} ref={item.ref} />
    </View>
  {/if}
</ItemFrame>

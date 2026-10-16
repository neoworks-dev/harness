<script lang="ts">
  // The session's chat. For now each item is shown as its kind and ref; the real drawing of
  // every component comes next.
  import { List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { suite } from "../state/suite.svelte.ts";

  const p = $derived(suite.palette);
</script>

<List
  items={[...suite.items]}
  key={(item: ViewItem) => item.ref}
  stickToEnd
  style={{ flex: 1, minHeight: 0 }}
>
  {#snippet row(item: ViewItem)}
    <View style={{ paddingX: p.space.l, paddingY: p.space.s }}>
      <Text style={{ color: p.muted }}>{item.kind} · {item.component.type}</Text>
    </View>
  {/snippet}
</List>

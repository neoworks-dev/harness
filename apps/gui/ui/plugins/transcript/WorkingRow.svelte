<script lang="ts">
  // "Working for 12s" under a running turn, counting by the second.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { duration } from "./format.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import { softBorder } from "./layout.ts";
  import type { Row } from "./rows.ts";

  let { row }: { row: Row } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  $effect(() => clock.join());

  const label = $derived(
    row.kind !== "working" || row.startedAt === undefined
      ? "Working"
      : `Working for ${duration(clock.now - row.startedAt)}`,
  );
</script>

<ItemFrame gap={8}>
  <View style={{ paddingX: 4, paddingTop: 4, paddingBottom: 8, borderBottomWidth: 1, borderColor: softBorder(p), fontSize: 14, color: p.muted }}>
    <Text>{label}</Text>
  </View>
</ItemFrame>

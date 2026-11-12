<script lang="ts">
  // "Working for 12s" under a running turn, counting by the second. A turn left open only for
  // subagents in the background (the daemon's waitingOn) says that instead: the reply is done,
  // and what is sent now goes in at once.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { duration, workingLabel } from "./format.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import { softBorder } from "./layout.ts";
  import type { Row } from "./rows.ts";

  let { row }: { row: Row } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  $effect(() => clock.join());

  const label = $derived(
    workingLabel(
      daemon.session?.waitingOn,
      row.kind === "working" && row.startedAt !== undefined
        ? duration(clock.now - row.startedAt)
        : undefined,
    ),
  );
</script>

<ItemFrame gap={8}>
  <View style={{ paddingX: 4, paddingTop: 4, paddingBottom: 8, borderBottomWidth: 1, borderColor: softBorder(p), fontSize: 14, color: p.muted }}>
    <Text>{label}</Text>
  </View>
</ItemFrame>

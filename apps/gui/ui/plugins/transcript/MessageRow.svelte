<script lang="ts">
  // The agent's reply: its prose (drawn by `chat.item`), the files the turn changed when it is the
  // turn's last reply, and under them, while the pointer is over it, a button to copy it and its
  // time. The row keeps the room for those, so it does not change height as the pointer moves.
  import { View, Text } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import ChangedFiles from "./ChangedFiles.svelte";
  import { componentText } from "./components.ts";
  import CopyButton from "./CopyButton.svelte";
  import { clockTime } from "./format.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import type { Row } from "./rows.ts";

  let { row }: { row: Row } = $props();
  const it = $derived(row.kind === "message" ? row : undefined);

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let hover = $state(false);
</script>

{#if it}

<ItemFrame gap={12}>
  <View onhover={(h: boolean) => (hover = h)}>
    <Slot name="chat.item" key="agent" props={{ item: it.item }} />
    {#if it.final && it.changes.length > 0}
      <ChangedFiles files={it.changes} />
    {/if}
    <View style={{ direction: "row", align: "center", gap: 6, paddingX: 4, paddingTop: 4, opacity: hover ? 1 : 0, color: p.muted }}>
      <CopyButton text={componentText(it.item.component)} />
      {#if it.at !== undefined}
        <Text style={{ fontSize: 12 }}>{clockTime(it.at)}</Text>
      {/if}
    </View>
  </View>
</ItemFrame>
{/if}

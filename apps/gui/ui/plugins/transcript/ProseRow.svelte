<script lang="ts">
  // One top-level block of the agent's reply (rows.ts splitMessages), drawn as the reply draws its
  // text, so the reply reads as one; the guest's name, if any, over its first block. The pointer
  // over it counts as over the reply, for the copy button under the last block.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Blocks from "./Blocks.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { proseColor } from "./layout.ts";
  import type { Row } from "./rows.ts";
  import { transcript } from "./state.svelte.ts";

  let { row }: { row: Row } = $props();
  const it = $derived(row.kind === "prose" ? row : undefined);

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  function hover(over: boolean): void {
    if (!it) return;
    if (over) transcript.hovered = it.ref;
    else if (transcript.hovered === it.ref) transcript.hovered = undefined;
  }
</script>

{#if it}
<ItemFrame gap={0}>
  <View style={{ paddingTop: it.first ? 6 : 0, paddingX: 4, gap: p.space.s, color: proseColor(p), fontSize: p.text.body }} onhover={hover}>
    {#if it.label}
      <Text style={{ color: p.muted, fontSize: 12, fontWeight: "medium" }}>{it.label}</Text>
    {/if}
    <Blocks blocks={[it.block]} size={p.text.body} continued={!it.first} />
  </View>
</ItemFrame>
{/if}

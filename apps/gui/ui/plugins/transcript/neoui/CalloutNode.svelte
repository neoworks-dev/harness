<script lang="ts">
  // A highlighted note: a bar in the tone, the title in bold, markdown under it.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import Blocks from "../Blocks.svelte";
  import { tint, toneColor } from "./look.ts";
  import { blocks, str, tone as toneOf } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const color = $derived(toneColor(toneOf(node) ?? "accent", p));
  const title = $derived(str(node, "title"));
</script>

<View style={{ background: tint(p, color, 10), radius: p.radius.s, borderLeftWidth: 3, borderColor: color, paddingX: 14, paddingY: 10, gap: 4 }}>
  {#if title}
    <Text style={{ fontWeight: "semibold", color }}>{title}</Text>
  {/if}
  <Blocks blocks={blocks(node)} size={p.text.body} />
</View>

<script lang="ts">
  // A rounded card on the surface colour: `tone` colours its outline and title, `sunk` is the fill
  // without the outline, `clear` neither (just the title above the children).
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { CARD_PADDING, insideCard } from "./layout.ts";
  import Kids from "./Kids.svelte";
  import { outline, toneColor } from "./look.ts";
  import { nodes, str, tone as toneOf } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node, path, width, fill = false }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const variant = $derived(str(node, "variant"));
  const tone = $derived(toneOf(node));
  const title = $derived(str(node, "title"));
  const framed = $derived(variant !== "clear");
  const inner = $derived(framed ? insideCard(width) : width);
</script>

<View
  style={{
    background: framed ? p.surface : undefined,
    radius: p.radius.m,
    borderWidth: variant === "sunk" || !framed ? 0 : 1,
    borderColor: outline(p, tone),
    padding: framed ? CARD_PADDING : 0,
    gap: 10,
    grow: fill ? 1 : undefined,
  }}
>
  {#if title}
    <Text style={{ fontWeight: "semibold", color: tone === undefined ? p.fg : toneColor(tone, p) }}>{title}</Text>
  {/if}
  <Kids children={nodes(node, "children")} {path} width={inner} />
</View>

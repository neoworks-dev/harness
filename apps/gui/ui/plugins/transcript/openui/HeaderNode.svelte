<script lang="ts">
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { str } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const subtitle = $derived(str(node, "subtitle"));
</script>

<View style={{ gap: 2 }}>
  <Text style={{ fontSize: p.text.title + 2, fontWeight: "semibold", lineHeight: 24 }}>{str(node, "title") ?? ""}</Text>
  {#if subtitle}
    <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>{subtitle}</Text>
  {/if}
</View>

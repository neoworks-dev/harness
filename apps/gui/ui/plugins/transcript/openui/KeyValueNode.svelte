<script lang="ts">
  // Aligned lines: the key muted, the value after it, in its tone when it has one.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { toneColor } from "./look.ts";
  import { nodes, str, tone as toneOf } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node, width }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const pairs = $derived(nodes(node, "pairs"));
  const keyWidth = $derived(Math.min(Math.floor(width * 0.4), Math.max(70, ...pairs.map((pair) => (str(pair, "key") ?? "").length * 7.4 + 8))));
</script>

<View style={{ gap: 4 }}>
  {#each pairs as pair}
    <View style={{ direction: "row", gap: 12 }}>
      <Text style={{ width: keyWidth, shrink: 0, color: p.muted }}>{str(pair, "key") ?? ""}</Text>
      <Text style={{ flex: 1, minWidth: 0, color: toneColor(toneOf(pair), p), fontWeight: toneOf(pair) ? "medium" : "normal" }}>{str(pair, "value") ?? ""}</Text>
    </View>
  {/each}
</View>

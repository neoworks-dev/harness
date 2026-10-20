<script lang="ts">
  // A pill: the label in its tone on a wash of it, neutral without a tone.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { tint, toneColor } from "./look.ts";
  import { str, tone as toneOf } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const tone = $derived(toneOf(node));
  const color = $derived(tone === undefined ? p.fg : toneColor(tone, p));
</script>

<View style={{ alignSelf: "start", paddingX: 9, paddingY: 2, radius: 999, background: tone === undefined ? p.raised : tint(p, color, 18) }}>
  <Text style={{ fontSize: 12, fontWeight: "medium", color, nowrap: true }}>{str(node, "label") ?? ""}</Text>
</View>

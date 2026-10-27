<script lang="ts">
  // A link: opened in the system's browser when pressed.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { useDoc } from "./context.ts";
  import { str } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const doc = useDoc();
  const url = $derived(str(node, "url") ?? "");
</script>

<View onpress={() => doc.open(url)} style={{ alignSelf: "start", cursor: "pointer" }}>
  <Text style={{ color: p.accent, underline: true }}>{str(node, "text") || url}</Text>
</View>

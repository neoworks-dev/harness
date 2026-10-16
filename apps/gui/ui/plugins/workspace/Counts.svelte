<script lang="ts">
  // "+12 −3": the lines added and removed, in the diff's colours.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { shortCount } from "./workspace.ts";

  let { added, deleted }: { added: number; deleted: number } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View style={{ direction: "row", gap: 6, shrink: 0 }}>
  <Text style={{ fontFamily: "mono", fontSize: 11, color: p.success }}>+{shortCount(added)}</Text>
  <Text style={{ fontFamily: "mono", fontSize: 11, color: p.danger }}>−{shortCount(deleted)}</Text>
</View>

<script lang="ts">
  // The line that opens and closes the settled sessions: "Settled (3)", a thin line across and a
  // chevron.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/bold/caret-down-bold.svg";
  import caretRight from "@phosphor-icons/core/bold/caret-right-bold.svg";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { count, open, onpress }: { count: number; open: boolean; onpress: () => void } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View
  {onpress}
  hover={{ color: p.fg }}
  style={{
    direction: "row",
    align: "center",
    gap: 8,
    height: 32,
    paddingX: 10,
    marginTop: 4,
    color: p.muted,
    cursor: "pointer",
    transition: 120,
  }}
>
  <Text style={{ fontSize: 12, fontWeight: "medium" }}>Settled ({count})</Text>
  <View style={{ flex: 1, height: 1, background: p.border }} />
  <Icon svg={open ? caretDown : caretRight} size={12} />
</View>

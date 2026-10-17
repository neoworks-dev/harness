<script lang="ts">
  // "Worked for 3m 43s ›": the header of a finished turn's work, which is hidden until it is
  // opened. What was opened is kept for the turn while the app runs.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import { softBorder } from "./layout.ts";
  import { opened } from "./opened.svelte.ts";
  import type { Row } from "./rows.ts";

  let { row }: { row: Row } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

{#if row.kind === "fold"}
<ItemFrame gap={12}>
  <View
    onpress={() => opened.toggle(`${daemon.sessionId}/${row.foldKey}`)}
    hover={{ color: p.fg }}
    style={{
      direction: "row",
      align: "center",
      gap: 4,
      paddingX: 4,
      paddingBottom: 8,
      borderBottomWidth: 1,
      borderColor: softBorder(p),
      fontSize: 14,
      color: p.muted,
      cursor: "pointer",
    }}
  >
    <Text>{row.label}</Text>
    <Icon svg={row.open ? caretDown : caretRight} size={14} />
  </View>
</ItemFrame>
{/if}

<script lang="ts">
  // A row that is one item as it is: drawn by whatever `chat.item` has for its kind.
  import Slot from "../../kernel/slots/Slot.svelte";
  import type { Row } from "./rows.ts";

  let { row }: { row: Row } = $props();
</script>

{#if row.kind === "item"}
  <Slot name="chat.item" key={row.item.kind} props={{ item: row.item }} />
{/if}

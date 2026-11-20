<script lang="ts">
  // The columns beside the tab's own session, left to right in the order they were opened.
  import { View } from "@neoworks/svelte-gpui/ui";
  import Column from "./Column.svelte";
  import { columns } from "./state.svelte.ts";
</script>

{#if columns.ids.length > 0}
  <View style={{ direction: "row", flex: columns.ids.length, minWidth: 0 }}>
    {#each columns.ids as session (session)}
      <Column {session} />
    {/each}
  </View>
{/if}

<script lang="ts">
  // The room around one chat item, whatever it is: the conversation's column, and space below.
  import type { Snippet } from "svelte";
  import Column from "../../widgets/Column.svelte";

  let { children, gap = 16 }: { children: Snippet; gap?: number } = $props();
</script>

<Column style={{ paddingBottom: gap }}>
  {@render children()}
</Column>

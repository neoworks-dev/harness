<script lang="ts">
  // The room around one chat item, whatever it is: the conversation's column, and space below.
  // The column gives the content its width in pixels, and at once: a row that waited to measure
  // its width first was laid out empty, and the list took those few pixels for its height.
  // A row that is made of items (a reply with the files it changed) frames them once.
  import { getContext, setContext, type Snippet } from "svelte";
  import Column from "../../widgets/Column.svelte";
  import { FRAMED } from "./layout.ts";
  import { chatZoom } from "./zoom.svelte.ts";

  let { children, gap = 16 }: { children: Snippet; gap?: number } = $props();

  const framed = getContext<boolean | undefined>(FRAMED) === true;
  setContext(FRAMED, true);
</script>

{#if framed}
  {@render children()}
{:else}
  <Column style={{ paddingBottom: gap }} zoom={chatZoom.value}>
    {@render children()}
  </Column>
{/if}

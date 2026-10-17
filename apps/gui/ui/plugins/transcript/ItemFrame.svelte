<script lang="ts">
  // The room around one chat item, whatever it is: the conversation's column, and space below.
  // The content gets the column's width in pixels, measured, and not a share of it: text in a
  // box with a percentage width is measured by the list at another width than it is drawn at.
  // A row that is made of items (a reply with the files it changed) frames them once.
  import { getContext, setContext, type Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import Column from "../../widgets/Column.svelte";
  import { FRAMED } from "./layout.ts";

  let { children, gap = 16 }: { children: Snippet; gap?: number } = $props();

  const framed = getContext<boolean | undefined>(FRAMED) === true;
  setContext(FRAMED, true);

  let width = $state(0);
</script>

{#if framed}
  {@render children()}
{:else}
  <Column style={{ paddingBottom: gap }}>
    <View onlayout={(box: Layout) => (width = box.width)}>
      {#if width > 0}
        <View style={{ width }}>
          {@render children()}
        </View>
      {/if}
    </View>
  </Column>
{/if}

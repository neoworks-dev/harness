<script lang="ts">
  // One surface's doc, drawn by the chat's neoui renderer in as much width as its place gives it
  // (the renderer lays out by the column's width, which a panel or page has to provide).
  import { setContext } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { COLUMN, useColumn, type ColumnSize } from "../../widgets/column.ts";
  import NeoUi from "../transcript/neoui/NeoUi.svelte";

  let { surface, own = false }: { surface: ViewItem; own?: boolean } = $props();

  /** The width measured, for a place that is not in the chat's column (`own`). */
  let width = $state(0);
  const parent = useColumn();
  const size: ColumnSize = {
    get width() {
      return own ? width || 320 : parent.width;
    },
    get zoom() {
      return own ? 1 : parent.zoom;
    },
  };
  setContext(COLUMN, size);
</script>

{#if surface.component.type === "neoui"}
  {#if own}
    <View onresize={(box: Layout) => (width = box.width)}>
      {#if width > 0}<NeoUi ui={surface.component.ui} ref={surface.ref} />{/if}
    </View>
  {:else}
    <NeoUi ui={surface.component.ui} ref={surface.ref} />
  {/if}
{/if}

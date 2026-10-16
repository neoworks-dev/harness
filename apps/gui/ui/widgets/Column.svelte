<script lang="ts">
  // The conversation's column: centred, at most 736 pixels wide, with room at the sides. The chat,
  // the input and the rows under it line up in it.
  //
  // The column's width is set in pixels from the width it is given, not as a share of it: inside a
  // list row (placed absolutely) a percentage width is resolved late, so text was measured wider
  // than it is drawn and the row came out shorter than its text.
  import type { Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Layout, Style } from "@neoworks/svelte-gpui/ui";

  let { children, style = {} }: { children: Snippet; style?: Style } = $props();

  /** The column's widest, and the room either side of it on a wide window. */
  const MAX_WIDTH = 736;
  const SIDE = 48;

  let given = $state(0);
  const width = $derived(Math.max(0, Math.min(MAX_WIDTH, given - SIDE * 2)));
</script>

<View style={{ align: "center", ...style }} onlayout={(box: Layout) => (given = box.width)}>
  {#if given > 0}
    <View style={{ width }}>
      {@render children()}
    </View>
  {/if}
</View>

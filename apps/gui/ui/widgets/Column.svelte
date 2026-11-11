<script lang="ts">
  // The conversation's column: centred, at most 736 pixels wide, with room at the sides. The chat,
  // the input and the rows under it line up in it.
  //
  // The column's width is set in pixels from the width it is given, not as a share of it: inside a
  // list row (placed absolutely) a percentage width is resolved late, so text was measured wider
  // than it is drawn and the row came out shorter than its text. The children draw at once, at the
  // widest, so a row isn't built twice as it scrolls in; on a wide window the width never changes.
  import { setContext, type Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Layout, Style } from "@neoworks/svelte-gpui/ui";
  import { COLUMN } from "./column.ts";

  let { children, style = {} }: { children: Snippet; style?: Style } = $props();

  /** The column's widest, and the room either side of it on a wide window. */
  const MAX_WIDTH = 736;
  const SIDE = 48;

  /** The width the column is given; until it is known, the column draws at its widest. */
  let given = $state<number>();
  const width = $derived(
    given === undefined ? MAX_WIDTH : Math.max(0, Math.min(MAX_WIDTH, given - SIDE * 2)),
  );

  setContext(COLUMN, {
    get width() {
      return width;
    },
  });
</script>

<!-- onresize, not onlayout: only the width matters, and every chat item has a column, so a move
     (the chat scrolling, an item above growing) must cost nothing. -->
<View style={{ align: "center", ...style }} onresize={(box: Layout) => (given = box.width)}>
  <View style={{ width }}>
    {@render children()}
  </View>
</View>

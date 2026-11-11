<script lang="ts">
  // The conversation's column: centred, at most 736 pixels wide, with room at the sides. The chat,
  // the input and the rows under it line up in it.
  //
  // The column's width is set in pixels from the width it is given, not as a share of it: inside a
  // list row (placed absolutely) a percentage width is resolved late, so text was measured wider
  // than it is drawn and the row came out shorter than its text. The children draw at once, at the
  // widest, so a row isn't built twice as it scrolls in; on a wide window the width never changes.
  //
  // `zoom` draws the children larger (the chat's size) in the same room: they are laid out in a
  // width `zoom` times smaller, which the zoom brings back to the column's. What is measured is
  // measured outside it, in the window's pixels.
  import { setContext, type Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Layout, Style } from "@neoworks/svelte-gpui/ui";
  import { COLUMN } from "./column.ts";

  let { children, style = {}, zoom = 1 }: { children: Snippet; style?: Style; zoom?: number } = $props();

  /** The column's widest, and the room either side of it on a wide window. */
  const MAX_WIDTH = 736;
  const SIDE = 48;

  /** The width the column is given; until it is known, the column draws at its widest. */
  let given = $state<number>();
  /** The column on screen: as wide as at the default size, a little wider when zoomed, if there is room. */
  const shown = $derived(
    Math.min(MAX_WIDTH * zoom, given === undefined ? MAX_WIDTH * zoom : Math.max(0, given - SIDE * 2)),
  );
  /** The width the children are laid out in, before the zoom. */
  const width = $derived(shown / zoom);

  setContext(COLUMN, {
    get width() {
      return width;
    },
    get zoom() {
      return zoom;
    },
  });
</script>

<!-- onresize, not onlayout: only the width matters, and every chat item has a column, so a move
     (the chat scrolling, an item above growing) must cost nothing. -->
<View style={{ align: "center", ...style }} onresize={(box: Layout) => (given = box.width)}>
  <View style={zoom === 1 ? { width } : { width, zoom }}>
    {@render children()}
  </View>
</View>

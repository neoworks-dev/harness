<script lang="ts">
  // The conversation's column: centred, at most 736 pixels wide, with room at the sides. The chat,
  // the input and the rows under it line up in it.
  import type { Snippet } from "svelte";
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { Style } from "@neoworks/svelte-gpui/ui";

  let { children, style = {} }: { children: Snippet; style?: Style } = $props();

  /** The column's widest, and the room either side of it on a wide window. */
  const MAX_WIDTH = 736;
  const SIDE = 48;
</script>

<View style={{ align: "center", paddingX: SIDE, ...style }}>
  <View style={{ width: "full", maxWidth: MAX_WIDTH }}>
    {@render children()}
  </View>
</View>

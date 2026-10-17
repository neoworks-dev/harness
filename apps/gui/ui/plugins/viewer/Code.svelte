<script lang="ts">
  // The text of a line of code: 13px mono on one line, cut at the edge, or wrapped when the viewer's
  // word wrap is on.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import { LINE_HEIGHT } from "./rows.ts";
  import { wrappable } from "./look.ts";
  import { viewer } from "./state.svelte.ts";

  let { text, inset = 0 }: { text: string; inset?: number } = $props();

  const style = $derived({
    flex: 1,
    minWidth: 0,
    paddingLeft: inset,
    fontFamily: "mono",
    fontSize: 13,
    lineHeight: LINE_HEIGHT,
  });
</script>

{#if viewer.wrap}
  <Text {style}>{wrappable(text)}</Text>
{:else}
  <Text pre style={{ ...style, truncate: true }}>{text}</Text>
{/if}

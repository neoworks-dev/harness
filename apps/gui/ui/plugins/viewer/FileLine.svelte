<script lang="ts">
  // One line of a file tab: its number (pressing it selects the line for a comment) and its text.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import Code from "./Code.svelte";
  import { selected as isSelected } from "./selection.ts";
  import { fileView, viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    /** 0-based; the number shown is one more. */
    index: number;
    text: string;
    /** The width of the numbers' column. */
    width: number;
  }

  let { path, index, text, width }: Props = $props();

  const { daemon, comments } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
  const view = $derived(fileView(path));
  const on = $derived(isSelected(viewer.selection, view, index));
  const marked = $derived(comments.marked(path, "new", index + 1));
</script>

<View
  style={{
    direction: "row",
    minHeight: LINE_HEIGHT,
    height: viewer.wrap ? undefined : LINE_HEIGHT,
    background: on ? colors.selected : "transparent",
  }}
>
  <View
    onpress={(event) => viewer.press(view, index, event.shift)}
    style={{ shrink: 0, background: on ? colors.selectedNumbers : colors.numbers, cursor: "pointer" }}
  >
    <Text
      style={{ width, textAlign: "right", paddingRight: 8, fontFamily: "mono", fontSize: 12, lineHeight: LINE_HEIGHT, color: p.muted }}
    >{index + 1}</Text>
  </View>
  <Code {text} inset={10} />
  {#if marked}
    <View style={{ position: "absolute", left: 3, top: 7, size: 6, radius: 3, background: p.accent }} />
  {/if}
</View>

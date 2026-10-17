<script lang="ts">
  // One line of a diff, unified: the old and the new line number in a gutter (pressing it selects
  // the line for a comment), the mark and the text, on the green or red of a change.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { DiffLine } from "@suite/protocol/types";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Code from "./Code.svelte";
  import { selected as isSelected } from "./selection.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import { backgrounds, markOf, numberWidth } from "./look.ts";
  import { diffView, viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    index: number;
    line: DiffLine;
  }

  let { path, index, line }: Props = $props();

  const { daemon, comments } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
  const view = $derived(diffView(path));
  const on = $derived(isSelected(viewer.selection, view, index));
  const look = $derived(backgrounds(colors, line.kind, on));
  const width = $derived(numberWidth(viewer.digits));
  const marked = $derived(
    line.kind === "remove"
      ? line.old !== undefined && comments.marked(path, "old", line.old)
      : line.new !== undefined && comments.marked(path, "new", line.new),
  );
  const number = $derived({
    width,
    textAlign: "right" as const,
    paddingRight: 8,
    fontFamily: "mono",
    fontSize: 12,
    lineHeight: LINE_HEIGHT,
    color: p.muted,
  });
</script>

<View
  style={{
    direction: "row",
    minHeight: LINE_HEIGHT,
    height: viewer.wrap ? undefined : LINE_HEIGHT,
    background: look.body,
  }}
>
  <View
    onpress={(event) => viewer.press(view, index, event.shift)}
    style={{ direction: "row", shrink: 0, background: look.numbers, cursor: "pointer" }}
  >
    <Text style={number}>{line.old ?? ""}</Text>
    <Text style={number}>{line.new ?? ""}</Text>
  </View>
  <Text
    style={{
      width: 18,
      shrink: 0,
      textAlign: "center",
      fontFamily: "mono",
      fontSize: 13,
      lineHeight: LINE_HEIGHT,
      color: line.kind === "add" ? p.success : p.danger,
    }}
  >{markOf(line.kind)}</Text>
  <Code text={line.text} />
  {#if marked}
    <View style={{ position: "absolute", left: 3, top: 7, size: 6, radius: 3, background: p.accent }} />
  {/if}
</View>

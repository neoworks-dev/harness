<script lang="ts">
  // One half of a split row: a line number, the text and the change's colour; or nothing, when the
  // other side has more lines.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Code from "./Code.svelte";
  import { backgrounds, markOf, numberWidth } from "./look.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import { selected as isSelected } from "./selection.ts";
  import type { Cell } from "./split.ts";
  import type { Token } from "./syntax.ts";
  import { diffView, viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    cell?: Cell;
    side: "old" | "new";
    tokens?: Token[];
  }

  let { path, cell, side, tokens }: Props = $props();

  const { daemon, comments } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
  const view = $derived(diffView(path));
  const line = $derived(cell?.line);
  const no = $derived(side === "old" ? line?.old : line?.new);
  const on = $derived(cell !== undefined && isSelected(viewer.selection, view, cell.index));
  const look = $derived(backgrounds(colors, line?.kind ?? "context", on));
  const marked = $derived(no !== undefined && comments.marked(path, side, no));
</script>

{#if cell === undefined || line === undefined}
  <View style={{ flex: 1, minWidth: 0, background: colors.empty }} />
{:else}
  <View style={{ direction: "row", flex: 1, minWidth: 0, background: look.body }}>
    <View
      onpress={(event) => viewer.press(view, cell.index, event.shift)}
      style={{ shrink: 0, background: look.numbers, cursor: "pointer" }}
    >
      <Text
        style={{
          width: numberWidth(viewer.digits),
          textAlign: "right",
          paddingRight: 8,
          fontFamily: "mono",
          fontSize: 12,
          lineHeight: LINE_HEIGHT,
          color: p.muted,
        }}
      >{no ?? ""}</Text>
    </View>
    <Text
      style={{
        width: 16,
        shrink: 0,
        textAlign: "center",
        fontFamily: "mono",
        fontSize: 13,
        lineHeight: LINE_HEIGHT,
        color: line.kind === "add" ? p.success : p.danger,
      }}
    >{markOf(line.kind)}</Text>
    <Code text={line.text} {tokens} />
    {#if marked}
      <View style={{ position: "absolute", left: 3, top: 7, size: 6, radius: 3, background: p.accent }} />
    {/if}
  </View>
{/if}

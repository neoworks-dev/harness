<script lang="ts">
  // One row of a split diff: the old line on the left, the new on the right, each with its own line
  // number (pressing it selects the line for a comment).
  import { View } from "@neoworks/svelte-gpui/ui";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import type { Cell } from "./split.ts";
  import SideCell from "./SideCell.svelte";
  import { viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    left?: Cell;
    right?: Cell;
  }

  let { path, left, right }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
</script>

<View style={{ direction: "row", minHeight: LINE_HEIGHT, height: viewer.wrap ? undefined : LINE_HEIGHT }}>
  <SideCell {path} cell={left} side="old" />
  <View style={{ width: 1, background: colors.numbers }} />
  <SideCell {path} cell={right} side="new" />
</View>

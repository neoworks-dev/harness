<script lang="ts">
  // One row of a split diff: the old line on the left, the new on the right, each with its own line
  // number (pressing it selects the line for a comment).
  import { View } from "@neoworks/svelte-gpui/ui";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import type { Cell } from "./split.ts";
  import type { Token } from "./syntax.ts";
  import SideCell from "./SideCell.svelte";
  import { viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    left?: Cell;
    right?: Cell;
    leftTokens?: Token[];
    rightTokens?: Token[];
  }

  let { path, left, right, leftTokens, rightTokens }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
  /** A removed line opposite an added one: each shows what changed against the other. */
  const rewritten = $derived(left?.line.kind === "remove" && right?.line.kind === "add");
</script>

<View style={{ direction: "row", minHeight: LINE_HEIGHT, height: viewer.wrap ? undefined : LINE_HEIGHT }}>
  <SideCell {path} cell={left} side="old" tokens={leftTokens} partner={rewritten ? right?.line.text : undefined} />
  <View style={{ width: 1, background: colors.numbers }} />
  <SideCell {path} cell={right} side="new" tokens={rightTokens} partner={rewritten ? left?.line.text : undefined} />
</View>

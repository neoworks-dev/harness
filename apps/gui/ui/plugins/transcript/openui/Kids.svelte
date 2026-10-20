<script lang="ts">
  // The children of a column Stack, a Card, a Tab or an accordion item. A run of sibling Cards is
  // drawn as a grid and a run of Metrics as a strip, whatever the program says, as in the terminal.
  import { View } from "@neoworks/svelte-gpui/ui";
  import type { UiNode } from "@suite/protocol/types";
  import Cells from "./Cells.svelte";
  import { childPath } from "./focus.ts";
  import { columnsFor, GRID_MIN, segments } from "./layout.ts";
  import MetricStrip from "./MetricStrip.svelte";
  import Node from "./Node.svelte";

  interface Props {
    children: UiNode[];
    path: string;
    width: number;
    /** The prop the children are under, for their paths. */
    prop?: string;
    gap?: number;
    align?: "start" | "center" | "end" | "stretch";
  }

  let { children, path, width, prop = "children", gap = 12, align = "stretch" }: Props = $props();
  const parts = $derived(segments(children));
</script>

<View style={{ gap, align }}>
  {#each parts as part}
    {#if part.kind === "node"}
      <Node node={part.at.node} path={childPath(path, prop, part.at.index)} {width} />
    {:else if part.kind === "cards"}
      <Cells items={part.items} columns={columnsFor(width, part.items.length, GRID_MIN, gap)} {width} {gap} {path} {prop} />
    {:else}
      <MetricStrip items={part.items} {width} {path} {prop} />
    {/if}
  {/each}
</View>

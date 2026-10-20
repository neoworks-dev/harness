<script lang="ts">
  // A column of children, or a row: the children keep their own width, shrink towards their
  // narrowest and wrap with `wrap`; a row that does not fit at the narrowest stacks.
  import { View } from "@neoworks/svelte-gpui/ui";
  import { childPath } from "./focus.ts";
  import { gapPx, isInline, isRow, rowFits } from "./layout.ts";
  import Kids from "./Kids.svelte";
  import MetricStrip from "./MetricStrip.svelte";
  import Node from "./Node.svelte";
  import { bool, nodes, str } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node, path, width }: NodeProps = $props();

  const children = $derived(nodes(node, "children"));
  const gap = $derived(gapPx(str(node, "gap")));
  const wrap = $derived(bool(node, "wrap") === true);
  const align = $derived(str(node, "align"));
  const justify = $derived(str(node, "justify"));
  const row = $derived(isRow(node) && (wrap || rowFits(children, width, gap)));
  const alignment = $derived(
    align === "start" || align === "center" || align === "end" || align === "stretch" ? align : undefined,
  );
  /** The width a child that shares the row gets, for its own layout. */
  const share = $derived(Math.max(0, Math.floor((width - gap * (children.length - 1)) / Math.max(1, children.length))));
</script>

{#if isRow(node) && children.length >= 2 && children.every((child) => child.component === "Metric")}
  <MetricStrip items={children.map((child, index) => ({ node: child, index }))} {width} {path} />
{:else if row}
  <View
    style={{
      direction: "row",
      wrap,
      gap,
      align: alignment ?? (wrap ? "center" : "stretch"),
      justify: justify === "start" || justify === "center" || justify === "end" || justify === "between" ? justify : undefined,
    }}
  >
    {#each children as child, index}
      {#if isInline(child)}
        <View style={{ shrink: child.component === "Text" || child.component === "Header" ? 1 : 0, minWidth: 0 }}>
          <Node node={child} path={childPath(path, "children", index)} width={share} />
        </View>
      {:else}
        <View style={{ flex: 1, minWidth: 0 }}>
          <Node node={child} path={childPath(path, "children", index)} width={share} fill />
        </View>
      {/if}
    {/each}
  </View>
{:else}
  <Kids {children} {path} {width} {gap} align={alignment} />
{/if}

<script lang="ts">
  // A bulleted or numbered list of text or components.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { marker } from "./content.ts";
  import { childPath } from "./focus.ts";
  import Node from "./Node.svelte";
  import { bool, cellOf, list } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node, path, width }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const items = $derived(list(node, "items").map(cellOf));
  const ordered = $derived(bool(node, "ordered") === true);
</script>

<View style={{ gap: 4 }}>
  {#each items as item, index}
    <View style={{ direction: "row", gap: 8 }}>
      <Text style={{ width: ordered ? 22 : 12, shrink: 0, color: p.muted, textAlign: ordered ? "right" : "left" }}>{marker(ordered, index)}</Text>
      {#if typeof item === "string"}
        <Text style={{ flex: 1, minWidth: 0 }}>{item}</Text>
      {:else}
        <View style={{ flex: 1, minWidth: 0 }}>
          <Node node={item} path={childPath(path, "items", index)} width={width - 30} />
        </View>
      {/if}
    </View>
  {/each}
</View>

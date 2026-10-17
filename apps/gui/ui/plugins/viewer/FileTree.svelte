<script lang="ts">
  // The changed files as a tree beside the diff: folders that fold, each file with what it changed
  // (the one shown is marked), and a press on a file takes the diff to it. The header counts the
  // files and folds every folder.
  import { untrack } from "svelte";
  import { List, Text, View } from "@neoworks/svelte-gpui/ui";
  import arrowsIn from "@phosphor-icons/core/regular/arrows-in-line-vertical.svg";
  import arrowsOut from "@phosphor-icons/core/regular/arrows-out-line-vertical.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { folderPaths, TREE_ROW_HEIGHT, treeRows, type TreeRow } from "./filetree.ts";
  import IconButton from "./IconButton.svelte";
  import { viewer } from "./state.svelte.ts";
  import TreeItem from "./TreeItem.svelte";

  let { width }: { width: number } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const rows = $derived(treeRows(viewer.diffs, viewer.closedDirs));
  const folders = $derived(folderPaths(viewer.diffs));
  const allClosed = $derived(folders.length > 0 && folders.every((path) => viewer.closedDirs.has(path)));
  let list: ReturnType<typeof List<TreeRow>> | undefined = $state();

  // The file shown changes as the diff scrolls: keep it in the tree's view.
  $effect(() => {
    const path = viewer.current;
    if (path === undefined) return;
    const at = rows.findIndex((row) => row.kind === "file" && row.path === path);
    if (at >= 0) untrack(() => list?.scrollToIndex(at, { align: "nearest" }));
  });
</script>

<View style={{ width, shrink: 0, borderRightWidth: 1, borderColor: p.border }}>
  <View style={{ direction: "row", align: "center", gap: 6, height: 32, paddingLeft: 12, paddingRight: 8, shrink: 0 }}>
    <Text style={{ fontSize: 12, fontWeight: "medium" }}>Files</Text>
    <Text style={{ fontSize: 11, color: p.muted, flex: 1 }}>{viewer.diffs.length}</Text>
    {#if folders.length > 0}
      <IconButton
        svg={allClosed ? arrowsOut : arrowsIn}
        label={allClosed ? "Open all folders" : "Fold all folders"}
        size={24}
        onpress={() => (viewer.closedDirs = allClosed ? new Set() : new Set(folders))}
      />
    {/if}
  </View>
  <List bind:this={list} items={rows} rowHeight={TREE_ROW_HEIGHT} key={(row) => row.key} style={{ flex: 1, minHeight: 0 }}>
    {#snippet row(entry: TreeRow)}
      <TreeItem row={entry} />
    {/snippet}
  </List>
</View>

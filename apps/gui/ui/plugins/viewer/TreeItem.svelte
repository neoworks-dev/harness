<script lang="ts">
  // One row of the file tree: a folder (pressing it folds it) or a file (pressing it brings its diff
  // to the top), with its icon, its name, what it changed and a bar in the margin while it is the
  // file shown. An added file's name is green and a deleted one's red.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import folderIcon from "@phosphor-icons/core/regular/folder-simple.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import { TREE_ROW_HEIGHT, type TreeRow } from "./filetree.ts";
  import { viewer } from "./state.svelte.ts";

  let { row }: { row: TreeRow } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const on = $derived(row.kind === "file" && viewer.current === row.path);
  const tint = $derived.by(() => {
    if (row.kind !== "file") return p.fg;
    if (row.status === "added") return p.success;
    return row.status === "deleted" ? p.danger : p.fg;
  });
</script>

<View
  onpress={() => (row.kind === "dir" ? viewer.toggleDir(row.path) : viewer.goto(row.path))}
  style={{
    direction: "row",
    align: "center",
    gap: 6,
    height: TREE_ROW_HEIGHT,
    paddingLeft: 8 + row.depth * 12,
    paddingRight: 10,
    cursor: "pointer",
    background: on ? p.raised : "transparent",
  }}
  hover={{ background: on ? p.raised : p.hover }}
>
  {#if on}
    <View style={{ position: "absolute", left: 0, top: 4, bottom: 4, width: 2, radius: 1, background: p.accent }} />
  {/if}
  {#if row.kind === "dir"}
    <View style={{ width: 12, shrink: 0, color: p.muted }}>
      <Icon svg={row.open ? caretDown : caretRight} size={12} />
    </View>
    <View style={{ width: 14, shrink: 0, color: p.muted }}>
      <Icon svg={folderIcon} size={14} />
    </View>
    <Text style={{ fontSize: 12, flex: 1, minWidth: 0, truncate: true, color: p.muted }}>{row.name}</Text>
  {:else}
    <View style={{ width: 12, shrink: 0 }} />
    <FileIcon path={row.path} size={14} />
    <Text style={{ fontSize: 12, flex: 1, minWidth: 0, truncate: true, color: tint, fontWeight: on ? "medium" : "normal" }}>{row.name}</Text>
    <Text style={{ shrink: 0, fontFamily: "mono", fontSize: 10.5 }}>
      <Text style={{ color: p.success }}>{row.added > 0 ? `+${row.added}` : ""}</Text>
      <Text style={{ color: p.danger }}>{row.removed > 0 ? ` −${row.removed}` : ""}</Text>
    </Text>
  {/if}
</View>

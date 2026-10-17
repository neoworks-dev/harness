<script lang="ts">
  // A file's header in the diff: the chevron that folds it, its icon and path (the directory
  // muted), whether it is new or deleted, what it changed (the counts and a bar of squares) and a
  // button that copies its path. Pressing the name opens the file. Every file but the first has a
  // gap above its header.
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import CopyPath from "./CopyPath.svelte";
  import type { Status } from "./filetree.ts";
  import { statBar } from "./look.ts";
  import { FILE_GAP, HEADER_HEIGHT } from "./rows.ts";
  import { viewer } from "./state.svelte.ts";

  /** Under this width the header leaves out what it can do without. */
  const NARROW = 400;

  interface Props {
    path: string;
    added: number;
    removed: number;
    status: Status;
    collapsed: boolean;
    /** Room above it, for every file but the first. */
    gap?: boolean;
  }

  let { path, added, removed, status, collapsed, gap = false }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const slash = $derived(path.lastIndexOf("/"));
  const bar = $derived(statBar(added, removed));
  const shade = $derived({ add: p.success, remove: p.danger, none: p.border });
  let width = $state(NARROW);
  const narrow = $derived(width < NARROW);
</script>

<View style={{ paddingTop: gap ? FILE_GAP : 0 }} onresize={(box: Layout) => { width = box.width; }}>
  <View
    style={{
      direction: "row",
      align: "center",
      gap: 8,
      height: HEADER_HEIGHT,
      paddingLeft: 8,
      paddingRight: 10,
      background: p.surface,
      borderBottomWidth: 1,
      borderTopWidth: 1,
      borderColor: p.border,
    }}
  >
    <Button
      onpress={() => viewer.toggleFile(path)}
      style={{ size: 20, shrink: 0, align: "center", justify: "center", radius: 4, color: p.muted }}
      hover={{ background: p.hover, color: p.fg }}
    >
      <Icon svg={collapsed ? caretRight : caretDown} size={13} />
    </Button>
    <View
      onpress={() => viewer.openFile(path)}
      hover={{ background: p.hover }}
      style={{ direction: "row", align: "center", gap: 8, flex: 1, minWidth: 0, height: 26, radius: 4, paddingX: 4, cursor: "pointer", overflow: "hidden" }}
    >
      <FileIcon {path} size={15} />
      <View style={{ direction: "row", flex: 1, minWidth: 0 }}>
        {#if slash >= 0}
          <Text style={{ fontSize: 12, lineHeight: 16, minWidth: 0, shrink: 1, truncate: true, color: p.muted }}>{path.slice(0, slash + 1)}</Text>
        {/if}
        <Text style={{ fontSize: 12, lineHeight: 16, shrink: 0, fontWeight: "medium" }}>{path.slice(slash + 1)}</Text>
      </View>
    </View>
    {#if status !== "modified" && !narrow}
      <View style={{ shrink: 0, paddingX: 6, height: 18, justify: "center", radius: 4, background: status === "added" ? p.success : p.danger, opacity: 0.9 }}>
        <Text style={{ fontSize: 10, fontWeight: "medium", color: p.bg }}>{status === "added" ? "New" : "Deleted"}</Text>
      </View>
    {/if}
    <Text style={{ shrink: 0, fontFamily: "mono", fontSize: 11 }}>
      <Text style={{ color: p.success }}>+{added}</Text>
      <Text style={{ color: p.danger }}> −{removed}</Text>
    </Text>
    {#if !narrow}
      <View style={{ direction: "row", gap: 2, shrink: 0 }}>
        {#each bar as square}
          <View style={{ size: 8, radius: 2, background: shade[square] }} />
        {/each}
      </View>
      <CopyPath {path} />
    {/if}
  </View>
</View>

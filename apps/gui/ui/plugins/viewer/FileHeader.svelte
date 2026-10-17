<script lang="ts">
  // A file's header in the diff: the chevron that folds it, its icon and path (the directory
  // muted), what it changed and a button that copies its path. Pressing the name opens the file.
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import CopyPath from "./CopyPath.svelte";
  import { HEADER_HEIGHT } from "./rows.ts";
  import { viewer } from "./state.svelte.ts";

  interface Props {
    path: string;
    added: number;
    removed: number;
    collapsed: boolean;
  }

  let { path, added, removed, collapsed }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const slash = $derived(path.lastIndexOf("/"));
</script>

<View
  style={{
    direction: "row",
    align: "center",
    gap: 8,
    minHeight: HEADER_HEIGHT,
    paddingTop: 6,
    paddingBottom: 6,
    paddingRight: 12,
    paddingLeft: 8,
    background: p.surface,
    borderBottomWidth: 1,
    borderTopWidth: 1,
    borderColor: p.border,
  }}
>
  <Button
    onpress={() => viewer.toggleFile(path)}
    style={{ size: 20, align: "center", justify: "center", radius: 4, color: p.muted }}
    hover={{ background: p.hover, color: p.fg }}
  >
    <Icon svg={collapsed ? caretRight : caretDown} size={13} />
  </Button>
  <View
    onpress={() => viewer.openFile(path)}
    hover={{ background: p.hover }}
    style={{ direction: "row", align: "center", gap: 8, flex: 1, minWidth: 0, radius: 4, paddingX: 4, cursor: "pointer" }}
  >
    <FileIcon {path} size={15} />
    <View style={{ direction: "row", flex: 1, minWidth: 0 }}>
      {#if slash >= 0}
        <Text style={{ fontSize: 12, lineHeight: 16, minWidth: 0, shrink: 1, truncate: true, color: p.muted }}>{path.slice(0, slash + 1)}</Text>
      {/if}
      <Text style={{ fontSize: 12, lineHeight: 16, shrink: 0 }}>{path.slice(slash + 1)}</Text>
    </View>
  </View>
  <Text style={{ fontFamily: "mono", fontSize: 11, color: p.success }}>+{added}</Text>
  <Text style={{ fontFamily: "mono", fontSize: 11, color: p.danger }}>−{removed}</Text>
  <CopyPath {path} />
</View>

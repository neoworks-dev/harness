<script lang="ts">
  // The diff tab's toolbar: which changes it lists (or the patch's title), how much they changed,
  // and refresh, fold all, unified or split, and word wrap.
  import { Select, Text, View } from "@neoworks/svelte-gpui/ui";
  import arrowsIn from "@phosphor-icons/core/regular/arrows-in-line-vertical.svg";
  import arrowsOut from "@phosphor-icons/core/regular/arrows-out-line-vertical.svg";
  import clockwise from "@phosphor-icons/core/regular/arrow-clockwise.svg";
  import columns from "@phosphor-icons/core/regular/columns.svg";
  import rows from "@phosphor-icons/core/regular/rows.svg";
  import treeIcon from "@phosphor-icons/core/regular/tree-structure.svg";
  import wrapIcon from "@phosphor-icons/core/regular/arrow-u-down-left.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import IconButton from "./IconButton.svelte";
  import { viewer, type Scope } from "./state.svelte.ts";

  const SCOPES: { value: Scope; label: string }[] = [
    { value: "uncommitted", label: "Uncommitted" },
    { value: "turn", label: "Latest turn" },
  ];

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const split = $derived(viewer.layout === "split");
  const patch = $derived(viewer.source.kind === "patch" ? viewer.source : undefined);
</script>

<View
  style={{
    direction: "row",
    align: "center",
    gap: 4,
    height: 36,
    paddingX: 8,
    shrink: 0,
    borderTopWidth: 1,
    borderBottomWidth: 1,
    borderColor: p.border,
  }}
>
  {#if viewer.diffs.length > 1}
    <IconButton svg={treeIcon} label="File tree" active={viewer.treeShown} onpress={() => viewer.toggleTree()} />
  {/if}
  {#if patch}
    <Text style={{ fontSize: 12, fontWeight: "medium", flex: 1, minWidth: 0, truncate: true, paddingLeft: 4 }}>{patch.title}</Text>
  {:else}
    <Select
      value={viewer.scope}
      options={SCOPES}
      onchange={(scope) => viewer.setScope(scope)}
      style={{ height: 26, paddingY: 0, paddingX: 8, minWidth: 148, fontSize: 12, background: p.raised, borderColor: p.border }}
      panelStyle={{ background: p.surface, borderColor: p.border, minWidth: 148 }}
    />
  {/if}
  <View style={{ direction: "row", align: "center", gap: 6, paddingX: 6, shrink: 0 }}>
    <Text style={{ fontFamily: "mono", fontSize: 11, color: p.success }}>+{viewer.totals.added}</Text>
    <Text style={{ fontFamily: "mono", fontSize: 11, color: p.danger }}>−{viewer.totals.removed}</Text>
  </View>
  {#if !patch}<View style={{ flex: 1 }} />{/if}
  <IconButton svg={clockwise} label="Refresh" disabled={patch !== undefined || viewer.busy} onpress={() => void viewer.refresh()} />
  <IconButton
    svg={viewer.allCollapsed ? arrowsOut : arrowsIn}
    label={viewer.allCollapsed ? "Expand all" : "Collapse all"}
    onpress={() => viewer.toggleAll()}
  />
  <IconButton
    svg={split ? rows : columns}
    label={split ? "Unified layout" : "Split layout"}
    onpress={() => (viewer.layout = split ? "unified" : "split")}
  />
  <IconButton svg={wrapIcon} label="Word wrap" active={viewer.wrap} onpress={() => (viewer.wrap = !viewer.wrap)} />
</View>

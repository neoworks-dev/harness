<script lang="ts">
  // The panel's header: the diff tab and the files opened from it as small raised tabs, then the
  // buttons that make the panel fill the conversation's place and close it.
  import { Button, Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import diffIcon from "@phosphor-icons/core/regular/git-diff.svg";
  import arrowsIn from "@phosphor-icons/core/regular/arrows-in-simple.svg";
  import arrowsOut from "@phosphor-icons/core/regular/arrows-out-simple.svg";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import IconButton from "./IconButton.svelte";
  import { viewer, type Tab } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  function label(tab: Tab): string {
    if (tab.kind === "diff") return "Diff";
    return tab.path.slice(tab.path.lastIndexOf("/") + 1);
  }
</script>

<View style={{ direction: "row", align: "center", gap: 8, height: 40, paddingX: 8, shrink: 0 }}>
  <Scroll horizontal style={{ flex: 1, minWidth: 0 }}>
    <View style={{ direction: "row", align: "center", gap: 4, height: 40 }}>
      {#each viewer.tabs as tab (tab.id)}
        {@const on = tab.id === viewer.active}
        <View
          onpress={() => viewer.activate(tab.id)}
          style={{
            direction: "row",
            align: "center",
            gap: 6,
            height: 24,
            maxWidth: 144,
            paddingLeft: 8,
            paddingRight: tab.kind === "diff" ? 8 : 3,
            radius: 6,
            shrink: 0,
            cursor: "pointer",
            color: on ? p.fg : p.muted,
            background: on ? p.raised : "transparent",
            borderWidth: 1,
            borderColor: on ? p.border : "transparent",
          }}
          hover={{ background: on ? p.raised : p.hover, color: p.fg }}
        >
          {#if tab.kind === "diff"}
            <Icon svg={diffIcon} size={14} />
          {:else}
            <FileIcon path={tab.path} size={14} />
          {/if}
          <Text style={{ fontSize: 12, minWidth: 0, truncate: true }}>{label(tab)}</Text>
          {#if tab.kind === "file"}
            <Button
              onpress={() => viewer.closeTab(tab.id)}
              style={{ size: 18, align: "center", justify: "center", radius: 4, color: p.muted }}
              hover={{ background: p.border, color: p.fg }}
            >
              <Icon svg={xIcon} size={11} />
            </Button>
          {/if}
        </View>
      {/each}
    </View>
  </Scroll>
  <IconButton
    svg={viewer.maximized ? arrowsIn : arrowsOut}
    label={viewer.maximized ? "Restore" : "Maximize"}
    onpress={() => (viewer.maximized = !viewer.maximized)}
  />
  <IconButton svg={xIcon} label="Close" onpress={() => viewer.close()} />
</View>

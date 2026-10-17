<script lang="ts">
  // What the panel holds, whether it sits beside the conversation or in its place: the tabs, and
  // the diff or the file of the one shown.
  import { View } from "@neoworks/svelte-gpui/ui";
  import DiffTab from "./DiffTab.svelte";
  import FileTab from "./FileTab.svelte";
  import Tabs from "./Tabs.svelte";
  import { viewer } from "./state.svelte.ts";

  const tab = $derived(viewer.tab);
</script>

<View style={{ flex: 1, minWidth: 0, minHeight: 0 }}>
  <Tabs />
  {#if tab.kind === "diff"}
    <DiffTab />
  {:else}
    {#key tab.id}
      <FileTab path={tab.path} />
    {/key}
  {/if}
</View>

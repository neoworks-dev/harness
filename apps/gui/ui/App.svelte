<script lang="ts">
  // The window: sessions on the left, then the chat, the input and the status line.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import Chat from "./chat/Chat.svelte";
  import DiffOverlay from "./chat/DiffOverlay.svelte";
  import Composer from "./input/Composer.svelte";
  import StatusLine from "./input/StatusLine.svelte";
  import AgentView from "./side/AgentView.svelte";
  import Agents from "./side/Agents.svelte";
  import GitHub from "./side/GitHub.svelte";
  import { agentView } from "./side/agentview.svelte.ts";
  import SidePanels from "./side/SidePanels.svelte";
  import Sessions from "./side/Sessions.svelte";
  import Keys from "./kernel/Keys.svelte";
  import { suite } from "./state/suite.svelte.ts";
  import { ui } from "./state/ui.svelte.ts";

  $effect(() => {
    void suite.start();
  });

  const p = $derived(suite.palette);
</script>

<Keys />

<View style={{ flex: 1, direction: "row", background: p.bg, color: p.fg, fontSize: p.text.body }}>
  <Sessions />
  <View style={{ flex: 1, minWidth: 0 }}>
    {#if suite.sessionId === undefined}
      <View style={{ flex: 1, align: "center", justify: "center", gap: 8 }}>
        {#if suite.error}
          <Text style={{ color: p.danger }}>{suite.error}</Text>
        {:else}
          <Text style={{ color: p.muted }}>Connecting to the suite daemon…</Text>
        {/if}
      </View>
    {:else}
      {#if ui.githubOpen}
        <GitHub />
      {:else if agentView.isOpen}
        <AgentView />
      {:else}
        <Chat />
      {/if}
      <Composer />
      <Agents />
      <StatusLine />
    {/if}
  </View>
  <SidePanels />
  <DiffOverlay />
</View>

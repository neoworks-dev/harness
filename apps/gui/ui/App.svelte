<script lang="ts">
  // The window: sessions on the left, then the chat, the input and the status line.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import Chat from "./chat/Chat.svelte";
  import Composer from "./input/Composer.svelte";
  import StatusLine from "./input/StatusLine.svelte";
  import { suite } from "./state/suite.svelte.ts";

  $effect(() => {
    void suite.start();
  });

  const p = $derived(suite.palette);
</script>

<View style={{ flex: 1, direction: "row", background: p.bg, color: p.fg, fontSize: p.text.body }}>
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
      <Chat />
      <Composer />
      <StatusLine />
    {/if}
  </View>
</View>

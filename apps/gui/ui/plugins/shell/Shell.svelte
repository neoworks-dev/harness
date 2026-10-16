<script lang="ts">
  // The window: columns left and right of the conversation, the conversation in the middle with
  // rows under it, and an overlay above. Until a session is shown there is only a message.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Column from "../../widgets/Column.svelte";

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
</script>

<View style={{ flex: 1, direction: "row", background: p.bg, color: p.fg, fontSize: p.text.body }}>
  <Slot name="shell.left" />
  <View style={{ flex: 1, minWidth: 0 }}>
    {#if daemon.sessionId === undefined}
      <View style={{ flex: 1, align: "center", justify: "center", gap: 8 }}>
        {#if daemon.error}
          <Text style={{ color: p.danger }}>{daemon.error}</Text>
        {:else}
          <Text style={{ color: p.muted }}>Connecting to the suite daemon…</Text>
        {/if}
      </View>
    {:else}
      <Slot name="shell.main" />
      <Column style={{ paddingBottom: 8 }}>
        <Slot name="shell.bottom" />
      </Column>
    {/if}
  </View>
  <Slot name="shell.right" />
  <Slot name="shell.overlay" />
</View>

<script lang="ts">
  // The window. The toolbars and side columns are its outside, all in one colour: the sessions at
  // the left, a bar over the conversation, the side panel and a strip of buttons at the right. The
  // conversation floats over them on a card with rounded corners at its left. Until a session is
  // shown there is only a message.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Column from "../../widgets/Column.svelte";

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
</script>

<View style={{ flex: 1, direction: "row", background: p.side.bg, color: p.fg, fontSize: p.text.body }}>
  <Slot name="shell.left" />
  <View style={{ flex: 1, minWidth: 0 }}>
    <View style={{ direction: "row", align: "center", gap: 8, height: 48, paddingX: 8, shrink: 0 }}>
      <Slot name="shell.top" />
      <View style={{ flex: 1 }} />
      <Slot name="shell.top.right" />
    </View>
    <View style={{ flex: 1, minHeight: 0, direction: "row", paddingBottom: 8 }}>
      <View
        style={{
          flex: 1,
          minWidth: 0,
          background: p.bg,
          radius: 16,
          borderWidth: 1,
          borderColor: p.border,
          overflow: "hidden",
          shadow: { y: 4, blur: 24, color: "rgba(0, 0, 0, 0.45)" },
        }}
      >
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
          <View style={{ position: "absolute", top: 12, right: 12 }}>
            <Slot name="shell.card" />
          </View>
        {/if}
      </View>
      <Slot name="shell.right" />
      <View style={{ width: 48, shrink: 0, align: "center", gap: 4, paddingTop: 4 }}>
        <Slot name="shell.toolbar" />
      </View>
    </View>
  </View>
  <Slot name="shell.overlay" />
</View>

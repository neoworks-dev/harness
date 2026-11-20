<script lang="ts">
  // The panel in the conversation's place, inside its card: the input stays under it, so comments
  // can be sent from here. It slides in over where the chat was, and a file opened from the chat
  // has a bar to go back (or Esc).
  import { Button, Icon, Presence, Text, View } from "@neoworks/svelte-gpui/ui";
  import { fade, fly } from "svelte/transition";
  import arrowLeft from "@phosphor-icons/core/regular/arrow-left.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Panel from "./Panel.svelte";
  import { viewer } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Presence in={[fly, { x: 48, duration: 180 }]} out={[fade, { duration: 120 }]} style={{ flex: 1, minHeight: 0 }}>
  {#if viewer.inPlace}
    <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, height: 36, shrink: 0 }}>
      <Button
        onpress={() => viewer.close()}
        style={{ direction: "row", align: "center", gap: 6, paddingX: 8, height: 26, radius: p.radius.s, color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Icon svg={arrowLeft} size={14} />
        <Text>Back to the chat</Text>
      </Button>
      <Text style={{ color: p.muted, fontSize: 12 }}>Esc</Text>
    </View>
  {/if}
  <View style={{ flex: 1, minHeight: 0 }}>
    <Panel />
  </View>
</Presence>

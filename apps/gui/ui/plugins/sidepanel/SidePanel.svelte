<script lang="ts">
  // The column at the right, shown while there is something in it: the plan above, the files
  // below, as the plugins that own them put them in.
  import { window } from "@neoworks/svelte-gpui";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import { sections } from "./state.svelte.ts";

  const WIDTH = 280;
  /** The column is only worth its width from here on. */
  const MIN_WINDOW_WIDTH = 900;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

{#if daemon.sessionId !== undefined && window.size.width >= MIN_WINDOW_WIDTH && sections.held}
  <View
    style={{
      width: WIDTH,
      shrink: 0,
      gap: p.space.l,
      padding: p.space.m,
      background: p.surface,
      borderLeftWidth: 1,
      borderColor: p.border,
    }}
  >
    <Slot name="sidepanel.section" />
  </View>
{/if}

<script lang="ts">
  // The column at the right, outside the conversation's card, shown while there is something in it
  // and the user hasn't hidden it, and no pane covers its place (the viewer): the plan and the
  // files, each on a card of its own.
  import { window } from "@neoworks/svelte-gpui";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import { covers, panel, sections } from "./state.svelte.ts";

  const WIDTH = 300;
  /** The column is only worth its width from here on. */
  const MIN_WINDOW_WIDTH = 900;

  const { daemon } = useCtx();
</script>

{#if daemon.sessionId !== undefined && window.size.width >= MIN_WINDOW_WIDTH && sections.held && !panel.hidden && !covers.held}
  <View style={{ width: WIDTH, shrink: 0, gap: 8, paddingLeft: 8 }}>
    <Slot name="sidepanel.section" />
  </View>
{/if}

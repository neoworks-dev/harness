<script lang="ts">
  // The panel beside the conversation, on a card like it, between it and the side panel. Its left
  // edge, in the gap, drags to resize it.
  import { window } from "@neoworks/svelte-gpui";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Panel from "./Panel.svelte";
  import { MIN_WIDTH, viewer } from "./state.svelte.ts";

  /** The window keeps this much besides the panel: the sessions, the toolbar and a conversation. */
  const KEEP = 640;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let drag = $state<{ x: number; width: number }>();
  let hot = $state(false);

  const widest = $derived(Math.max(MIN_WIDTH, window.size.width - KEEP));
  const width = $derived(Math.min(widest, Math.max(MIN_WIDTH, viewer.width)));
  const grip = $derived.by(() => {
    if (drag) return p.accent;
    return hot ? p.border : "transparent";
  });
</script>

{#if daemon.sessionId !== undefined && viewer.shown && !viewer.maximized}
  <View style={{ width, shrink: 0, direction: "row" }}>
    <View
      ondown={(event) => (drag = { x: event.windowX, width })}
      onmove={(event) => {
        if (drag) viewer.width = Math.min(widest, Math.max(MIN_WIDTH, drag.width + drag.x - event.windowX));
      }}
      onup={() => (drag = undefined)}
      onhover={(over) => (hot = over)}
      style={{ width: 8, shrink: 0, align: "center", justify: "center", cursor: "col-resize" }}
    >
      <View style={{ width: 2, height: 48, radius: 1, background: grip }} />
    </View>
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
      <Panel />
    </View>
  </View>
{/if}

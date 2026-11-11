<script lang="ts">
  // The terminal beside the conversation, on a card like it. Its left edge, in the gap, drags to
  // resize it.
  import { window } from "@neoworks/svelte-gpui";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { MIN_WIDTH, terminal } from "./state.svelte.ts";
  import Terminal from "./Terminal.svelte";

  /** The window keeps this much besides the panel: the sessions, the toolbar and a conversation. */
  const KEEP = 640;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let drag = $state<{ x: number; width: number }>();
  let hot = $state(false);

  const widest = $derived(Math.max(MIN_WIDTH, window.size.width - KEEP));
  const width = $derived(Math.min(widest, Math.max(MIN_WIDTH, terminal.width)));
  const grip = $derived.by(() => {
    if (drag) return p.accent;
    return hot ? p.border : "transparent";
  });
</script>

{#if daemon.sessionId !== undefined && terminal.shown}
  <View style={{ width, shrink: 0, direction: "row" }}>
    <View
      ondown={(event) => (drag = { x: event.windowX, width })}
      onmove={(event) => {
        if (drag) terminal.width = Math.min(widest, Math.max(MIN_WIDTH, drag.width + drag.x - event.windowX));
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
      <Terminal />
    </View>
  </View>
{/if}

<script lang="ts">
  // The small editor under the selected lines: what to say about them. Enter adds the comment, which
  // waits in the input as a chip and goes with the next message; Esc drops it.
  import { Button, Input, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { rangeLabel } from "../../lib/comments.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { EDITOR_HEIGHT } from "./rows.ts";
  import { viewer } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let input: ReturnType<typeof Input> | undefined = $state();
  const place = $derived(viewer.place);
  const empty = $derived(viewer.draft.trim() === "");

  // The editor has the keyboard from the moment it is drawn.
  $effect(() => {
    input?.focus();
  });

  function key(event: KeyEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      viewer.clearSelection();
    } else if (event.key === "Enter" && !event.shift && !event.alt) {
      event.preventDefault();
      viewer.submit();
    }
  }
</script>

<View style={{ height: EDITOR_HEIGHT, paddingX: 12, paddingY: 8, background: p.bg }}>
  <View
    style={{ flex: 1, minHeight: 0, gap: 6, padding: 8, radius: 8, background: p.surface, borderWidth: 1, borderColor: p.border }}
  >
    <Text style={{ fontSize: 11, color: p.muted, truncate: true }}>
      Comment on {place ? rangeLabel(place) : "the selected lines"}
    </Text>
    <Input
      bind:this={input}
      bind:value={viewer.draft}
      multiline
      rows={2}
      placeholder="Say something about these lines"
      style={{ fontSize: 13, color: p.fg, flex: 1, minHeight: 0 }}
      onkey={key}
    />
    <View style={{ direction: "row", justify: "end", gap: 6 }}>
      <Button
        onpress={() => viewer.clearSelection()}
        style={{ paddingX: 10, height: 24, radius: 6, align: "center", color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Text style={{ fontSize: 12 }}>Cancel</Text>
      </Button>
      <Button
        onpress={() => viewer.submit()}
        disabled={empty}
        style={{ paddingX: 10, height: 24, radius: 6, align: "center", background: p.accent, color: "#ffffff" }}
        hover={{ opacity: 0.9 }}
        disabledStyle={{ opacity: 0.5 }}
      >
        <Text style={{ fontSize: 12 }}>Comment</Text>
      </Button>
    </View>
  </View>
</View>

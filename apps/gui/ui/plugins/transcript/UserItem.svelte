<script lang="ts" module>
  /** The measured width of each message's longest line, by item, while the app runs. */
  const naturalWidths = new Map<string, number>();
</script>

<script lang="ts">
  // The user's message in a bubble at the right, as wide as its text up to 80% of the column. What
  // other plugins add (the app it came from) goes under it, in `chat.item.header`.
  //
  // The bubble gets a definite width: text in a box that sizes to its content is measured on one
  // line and keeps that height after it wraps, which the list then trusts and overlaps the next
  // row. So the longest line is measured on its own first, unwrapped and out of sight.
  import { untrack } from "svelte";
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Component from "./Component.svelte";
  import { plainLines } from "./components.ts";
  import ItemFrame from "./ItemFrame.svelte";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  /** The bubble's padding on each side. */
  const PADDING = 12;

  const lines = $derived(plainLines(item.component));
  const longest = $derived(
    lines === undefined ? undefined : lines.reduce((a, b) => (b.length > a.length ? b : a), ""),
  );
  // A row is built per item (the list is keyed by ref), so the item's ref doesn't change under it.
  let natural = $state<number | undefined>(untrack(() => naturalWidths.get(item.ref)));
  let column = $state(0);
  /** Keeps the measured width for when the row is built again as it scrolls back in. */
  function remember(measured: number): void {
    naturalWidths.set(item.ref, measured);
    natural = measured;
  }

  const width = $derived.by(() => {
    const most = Math.floor(column * 0.8);
    if (natural === undefined) return most;
    return Math.min(Math.ceil(natural) + PADDING * 2 + 1, most);
  });
</script>

<ItemFrame>
  <View style={{ gap: 4, paddingTop: 8 }} onlayout={(box: Layout) => (column = box.width)}>
    {#if longest !== undefined && natural === undefined}
      <View style={{ position: "absolute", top: 0, left: 0, opacity: 0 }}>
        <Text style={{ nowrap: true }} onlayout={(box: Layout) => remember(box.width)}>{longest}</Text>
      </View>
    {/if}
    <View style={{ direction: "row", justify: "end" }}>
      {#if column > 0}
        <View style={{ width, background: p.message, radius: p.radius.l, padding: PADDING }}>
          <Component component={item.component} ref={item.ref} />
        </View>
      {/if}
    </View>
    <View style={{ direction: "row", justify: "end", paddingRight: 4 }}>
      <Slot name="chat.item.header" props={{ item }} />
    </View>
  </View>
</ItemFrame>

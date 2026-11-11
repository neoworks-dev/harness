<script lang="ts" module>
  /** The measured width of each message's longest line, by item, while the app runs. */
  const naturalWidths = new Map<string, number>();
  /**
   * The chat column's width as the last message measured it. A row built again as it scrolls back
   * in starts from it: from 0 it would draw without its bubble until its own measure arrives a
   * frame later, and the list, which had its height, would move everything under it twice.
   */
  let lastColumn = 0;
</script>

<script lang="ts">
  // The user's message in a bubble at the right, as wide as its text up to 80% of the column. A
  // long one is cut to a few lines until "Show full message" is pressed. Under it, in
  // `chat.item.header`, what other plugins add (the app it came from), and, while the pointer is
  // over the message, when it was sent and a button to copy it.
  //
  // The bubble gets a definite width: text in a box that sizes to its content is measured on one
  // line and keeps that height after it wraps, which the list then trusts and overlaps the next
  // row. So the longest line is measured on its own first, unwrapped and out of sight.
  import { untrack } from "svelte";
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Component from "./Component.svelte";
  import { componentText, plainLines } from "./components.ts";
  import CopyButton from "./CopyButton.svelte";
  import { clockTime } from "./format.ts";
  import { chatZoom } from "./zoom.svelte.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import { opened } from "./opened.svelte.ts";
  import { timeline } from "./timeline.svelte.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  /** The bubble's padding on each side. */
  const PADDING = 12;
  /** A message longer than this many lines or characters is cut to `CUT` pixels. */
  const LONG_LINES = 8;
  const LONG_CHARS = 600;
  const CUT = 176;

  const lines = $derived(plainLines(item.component));
  const longest = $derived(
    lines === undefined ? undefined : lines.reduce((a, b) => (b.length > a.length ? b : a), ""),
  );
  // A row is built per item (the list is keyed by ref), so the item's ref doesn't change under it.
  let natural = $state<number | undefined>(untrack(() => naturalWidths.get(item.ref)));
  let column = $state(lastColumn);
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

  const text = $derived(componentText(item.component));
  const long = $derived(text.split("\n").length > LONG_LINES || text.length > LONG_CHARS);
  const key = $derived(`${daemon.sessionId}/msg:${item.ref}`);
  const cut = $derived(long && !opened.has(key));
  const sent = $derived(timeline.value.stamps.get(item.ref)?.first);
  let hover = $state(false);
</script>

<ItemFrame gap={4}>
  <!-- Measured in the window's pixels, used as widths under the chat's zoom: divided by it. -->
  <View style={{ gap: 4, paddingTop: 8 }} onresize={(box: Layout) => (column = lastColumn = box.width / chatZoom.value)} onhover={(h: boolean) => (hover = h)}>
    {#if longest !== undefined && natural === undefined}
      <View style={{ position: "absolute", top: 0, left: 0, opacity: 0 }}>
        <Text style={{ nowrap: true }} onlayout={(box: Layout) => remember(box.width / chatZoom.value)}>{longest}</Text>
      </View>
    {/if}
    <View style={{ direction: "row", justify: "end" }}>
      {#if column > 0}
        <View style={{ width, background: p.message, radius: p.radius.l, padding: PADDING, gap: 8 }}>
          <View style={{ maxHeight: cut ? CUT : undefined, overflow: "hidden" }}>
            <Component component={item.component} ref={item.ref} />
          </View>
          {#if long}
            <Button
              onpress={() => opened.toggle(key)}
              style={{ alignSelf: "start", fontSize: 12, color: p.muted }}
              hover={{ color: p.fg }}
            >
              <Text>{cut ? "Show full message" : "Show less"}</Text>
            </Button>
          {/if}
        </View>
      {/if}
    </View>
    <View style={{ direction: "row", justify: "end", align: "center", gap: 6, paddingRight: 4, minHeight: 22 }}>
      <Slot name="chat.item.header" props={{ item }} />
      <View style={{ direction: "row", align: "center", gap: 4, opacity: hover ? 1 : 0, color: p.muted }}>
        {#if sent !== undefined}
          <Text style={{ fontSize: 12 }}>{clockTime(sent)}</Text>
        {/if}
        <CopyButton {text} size={22} />
      </View>
    </View>
  </View>
</ItemFrame>

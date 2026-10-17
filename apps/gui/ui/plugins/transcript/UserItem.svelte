<script lang="ts">
  // The user's message in a bubble at the right, as wide as its text up to 80% of the column. A
  // long one is cut to a few lines until "Show full message" is pressed. Under it, in
  // `chat.item.header`, what other plugins add (the app it came from), and, while the pointer is
  // over the message, when it was sent and a button to copy it.
  //
  // The bubble gets a definite width: text in a box that sizes to its content is measured on one
  // line and keeps that height after it wraps, which the list then trusts and overlaps the next
  // row. So the longest line is measured on its own first, unwrapped and out of sight.
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import Component from "./Component.svelte";
  import { componentText, plainLines } from "./components.ts";
  import CopyButton from "./CopyButton.svelte";
  import { clockTime } from "./format.ts";
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
  let natural = $state<number>();
  let column = $state(0);
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
  <View style={{ gap: 4, paddingTop: 8 }} onlayout={(box: Layout) => (column = box.width)} onhover={(h: boolean) => (hover = h)}>
    {#if longest !== undefined}
      <View style={{ position: "absolute", top: 0, left: 0, opacity: 0 }}>
        <Text style={{ nowrap: true }} onlayout={(box: Layout) => (natural = box.width)}>{longest}</Text>
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
